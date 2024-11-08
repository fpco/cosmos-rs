use std::{
    ops::Deref,
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use tonic::{
    codegen::InterceptedService,
    transport::{Channel, ClientTlsConfig, Endpoint, Uri},
};

use crate::{
    error::{
        Action, BuilderError, ConnectionError, LastNodeError, NodeHealthLevel, QueryErrorDetails,
        SingleNodeHealthReport,
    },
    CosmosBuilder,
};

use super::{node_chooser::QueryResult, CosmosInterceptor};

/// Internal data structure containing gRPC clients.
#[derive(Clone)]
pub(crate) struct Node {
    node_inner: Arc<NodeInner>,
}

struct NodeInner {
    grpc_url: Arc<String>,
    is_fallback: bool,
    last_error: RwLock<Option<LastError>>,
    channel: InterceptedService<Channel, CosmosInterceptor>,
    query_count: RwLock<QueryCount>,
    max_decoding_message_size: usize,
}

#[derive(Default)]
pub(crate) struct QueryCount {
    pub(crate) first_request: Option<DateTime<Utc>>,
    pub(crate) total_query_count: u64,
    pub(crate) total_error_count: u64,
}

impl QueryCount {
    pub(crate) fn incr(&mut self, is_error: bool) {
        if self.first_request.is_none() {
            self.first_request = Some(Utc::now());
        }
        self.total_query_count += 1;
        if is_error {
            self.total_error_count += 1;
        }
    }
}

#[derive(Debug)]
struct LastError {
    error: Arc<String>,
    instant: Instant,
    timestamp: DateTime<Utc>,
    action: Option<Action>,
    /// How many network errors in a row have occurred?
    ///
    /// Gets reset each time there's a successful query, or a query that fails with a non-network reason.
    error_count: usize,
    /// Does this error deserve blocking the node entirely?
    blocked: bool,
}

impl LastError {
    fn node_health_level(&self) -> NodeHealthLevel {
        const NODE_ERROR_TIMEOUT: u64 = 30;

        // If enough time has passed since the error, ignore it.
        if self.instant.elapsed().as_secs() > NODE_ERROR_TIMEOUT {
            NodeHealthLevel::Unblocked { error_count: 0 }
        }
        // If the error is a blocking error, we don't allow even a single error
        // through. Check that first.
        else if self.blocked {
            NodeHealthLevel::Blocked
        } else {
            NodeHealthLevel::Unblocked {
                error_count: self.error_count,
            }
        }
    }
}

impl CosmosBuilder {
    pub(crate) fn make_node(
        &self,
        grpc_url: &Arc<String>,
        is_fallback: bool,
    ) -> Result<Node, BuilderError> {
        let grpc_endpoint =
            grpc_url
                .parse::<Endpoint>()
                .map_err(|source| BuilderError::InvalidGrpcUrl {
                    grpc_url: grpc_url.clone(),
                    source: source.into(),
                })?;

        let uri =
            Uri::try_from(grpc_url.clone().deref()).map_err(|op| BuilderError::InvalidUri {
                gprc_url: grpc_url.clone(),
                source: op,
            })?;
        // https://github.com/hyperium/tonic/issues/1033#issuecomment-1537239811
        let grpc_endpoint = grpc_endpoint.origin(uri);

        let grpc_endpoint =
            if let Some(http2_keep_alive_interval) = self.get_http2_keep_alive_interval() {
                grpc_endpoint.http2_keep_alive_interval(http2_keep_alive_interval)
            } else {
                grpc_endpoint
            };

        let grpc_endpoint = if let Some(keep_alive_while_idle) = self.get_keep_alive_while_idle() {
            grpc_endpoint.keep_alive_while_idle(keep_alive_while_idle)
        } else {
            grpc_endpoint
        };

        let grpc_endpoint = if let Some(rate_limit) = self.rate_limit() {
            grpc_endpoint.rate_limit(rate_limit, Duration::from_secs(rate_limit))
        } else {
            grpc_endpoint
        };

        let grpc_endpoint = if grpc_url.starts_with("https://") {
            grpc_endpoint
                .tls_config(ClientTlsConfig::new().with_native_roots())
                .map_err(|source| BuilderError::TlsConfig {
                    grpc_url: grpc_url.clone(),
                    source: source.into(),
                })?
        } else {
            grpc_endpoint
        };

        let grpc_channel = grpc_endpoint.connect_lazy();

        let referer_header = self.referer_header().map(|x| x.to_owned());

        let interceptor = CosmosInterceptor(referer_header.map(Arc::new));
        let channel = InterceptedService::new(grpc_channel, interceptor);
        let max_decoding_message_size = self.get_max_decoding_message_size();

        Ok(Node {
            node_inner: Arc::new(NodeInner {
                is_fallback,
                channel,
                grpc_url: grpc_url.clone(),
                last_error: RwLock::new(None),
                query_count: RwLock::new(QueryCount::default()),
                max_decoding_message_size,
            }),
        })
    }
}

pub(crate) type CosmosChannel = InterceptedService<Channel, CosmosInterceptor>;

impl Node {
    pub(crate) fn grpc_url(&self) -> &Arc<String> {
        &self.node_inner.grpc_url
    }

    pub(crate) fn set_broken(
        &self,
        err: impl FnOnce(Arc<String>) -> ConnectionError,
        details: &QueryErrorDetails,
    ) {
        let err = err(self.node_inner.grpc_url.clone());
        self.log_connection_error(err, details);
    }

    fn log_connection_error(&self, error: ConnectionError, details: &QueryErrorDetails) {
        let mut guard = self.node_inner.last_error.write();
        let old_error_count = guard.as_ref().map_or(0, |x| x.error_count);
        *guard = Some(LastError {
            error: error.to_string().into(),
            instant: Instant::now(),
            timestamp: Utc::now(),
            action: None,
            error_count: old_error_count + 1,
            blocked: details.is_blocked(),
        });
    }

    pub(super) fn log_query_result(&self, res: QueryResult) {
        self.node_inner.query_count.write().incr(match res {
            QueryResult::Success => false,
            QueryResult::NetworkError { .. } | QueryResult::OtherError => true,
        });
        let mut guard = self.node_inner.last_error.write();
        match res {
            QueryResult::Success | QueryResult::OtherError => {
                if let Some(error) = guard.as_mut() {
                    error.error_count = 0;
                }
            }
            QueryResult::NetworkError { err, action } => {
                let old_error_count = guard.as_ref().map_or(0, |x| x.error_count);
                *guard = Some(LastError {
                    error: err.to_string().into(),
                    instant: Instant::now(),
                    timestamp: Utc::now(),
                    action: Some(action),
                    error_count: old_error_count + 1,
                    blocked: err.is_blocked(),
                });
            }
        }
    }

    pub(crate) fn is_fallback(&self) -> bool {
        self.node_inner.is_fallback
    }

    pub(crate) fn node_health_level(&self) -> NodeHealthLevel {
        match &*self.node_inner.last_error.read() {
            None => NodeHealthLevel::Unblocked { error_count: 0 },
            Some(last_error) => last_error.node_health_level(),
        }
    }

    pub(crate) fn health_report(&self) -> SingleNodeHealthReport {
        let guard = self.node_inner.last_error.read();
        let last_error = guard.as_ref();
        let QueryCount {
            first_request,
            total_query_count,
            total_error_count,
        } = *self.node_inner.query_count.read();
        SingleNodeHealthReport {
            grpc_url: self.node_inner.grpc_url.clone(),
            is_fallback: self.node_inner.is_fallback,
            node_health_level: last_error
                .map_or(NodeHealthLevel::Unblocked { error_count: 0 }, |x| {
                    x.node_health_level()
                }),
            error_count: last_error.map_or(0, |last_error| last_error.error_count),
            last_error: last_error.map(|last_error| {
                let error = match &last_error.action {
                    Some(action) => Arc::new(format!(
                        "{} during action {}",
                        last_error.error.clone(),
                        action
                    )),
                    None => last_error.error.clone(),
                };
                LastNodeError {
                    timestamp: last_error.timestamp,
                    age: last_error.instant.elapsed(),
                    error,
                }
            }),
            first_request,
            total_query_count,
            total_error_count,
        }
    }

    pub(crate) fn auth_query_client(
        &self,
    ) -> cosmos_sdk_proto::cosmos::auth::v1beta1::query_client::QueryClient<CosmosChannel> {
        let client = cosmos_sdk_proto::cosmos::auth::v1beta1::query_client::QueryClient::new(
            self.node_inner.channel.clone(),
        );
        client.max_decoding_message_size(self.node_inner.max_decoding_message_size)
    }

    pub(crate) fn bank_query_client(
        &self,
    ) -> cosmos_sdk_proto::cosmos::bank::v1beta1::query_client::QueryClient<CosmosChannel> {
        let client = cosmos_sdk_proto::cosmos::bank::v1beta1::query_client::QueryClient::new(
            self.node_inner.channel.clone(),
        );
        client.max_decoding_message_size(self.node_inner.max_decoding_message_size)
    }

    pub(crate) fn wasm_query_client(
        &self,
    ) -> cosmos_sdk_proto::cosmwasm::wasm::v1::query_client::QueryClient<CosmosChannel> {
        let client = cosmos_sdk_proto::cosmwasm::wasm::v1::query_client::QueryClient::new(
            self.node_inner.channel.clone(),
        );
        client.max_decoding_message_size(self.node_inner.max_decoding_message_size)
    }

    pub(crate) fn tx_service_client(
        &self,
    ) -> cosmos_sdk_proto::cosmos::tx::v1beta1::service_client::ServiceClient<CosmosChannel> {
        let client = cosmos_sdk_proto::cosmos::tx::v1beta1::service_client::ServiceClient::new(
            self.node_inner.channel.clone(),
        );
        client.max_decoding_message_size(self.node_inner.max_decoding_message_size)
    }

    pub(crate) fn tendermint_client(
        &self,
    ) -> cosmos_sdk_proto::cosmos::base::tendermint::v1beta1::service_client::ServiceClient<
        CosmosChannel,
    > {
        let client =
            cosmos_sdk_proto::cosmos::base::tendermint::v1beta1::service_client::ServiceClient::new(
                self.node_inner.channel.clone(),
            );
        client.max_decoding_message_size(self.node_inner.max_decoding_message_size)
    }

    pub(crate) fn authz_query_client(
        &self,
    ) -> cosmos_sdk_proto::cosmos::authz::v1beta1::query_client::QueryClient<CosmosChannel> {
        let client = cosmos_sdk_proto::cosmos::authz::v1beta1::query_client::QueryClient::new(
            self.node_inner.channel.clone(),
        );
        client.max_decoding_message_size(self.node_inner.max_decoding_message_size)
    }

    pub(crate) fn epochs_query_client(
        &self,
    ) -> crate::osmosis::epochs::query_client::QueryClient<CosmosChannel> {
        crate::osmosis::epochs::query_client::QueryClient::new(self.node_inner.channel.clone())
    }

    pub(crate) fn txfees_query_client(
        &self,
    ) -> crate::osmosis::txfees::query_client::QueryClient<CosmosChannel> {
        crate::osmosis::txfees::query_client::QueryClient::new(self.node_inner.channel.clone())
    }
}
