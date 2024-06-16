use std::sync::Arc;

use rand::seq::SliceRandom;

use crate::{
    error::{Action, BuilderError, ConnectionError, NodeHealthReport, QueryErrorDetails},
    CosmosBuilder,
};

use super::node::Node;

#[derive(Clone)]
pub(super) struct NodeChooser {
    primary: Arc<Node>,
    fallbacks: Arc<[Node]>,
    /// How many errors in a row are allowed before we call a node unhealthy?
    pub(crate) allowed_error_count: usize,
}

impl NodeChooser {
    pub(super) fn new(builder: &CosmosBuilder) -> Result<Self, BuilderError> {
        Ok(NodeChooser {
            primary: Arc::new(builder.make_node(builder.grpc_url_arc(), false)?),
            fallbacks: builder
                .grpc_fallback_urls()
                .iter()
                .map(|fallback| builder.make_node(fallback, true))
                .collect::<Result<Vec<_>, _>>()?
                .into(),
            allowed_error_count: builder.get_allowed_error_count(),
        })
    }

    pub(super) fn choose_node(&self) -> Result<&Node, ConnectionError> {
        // First we try to find a node that has had no issues at all.
        // If that fails, then we go for the allowed error count.
        // Motivation: we previously had issues where we would retry the primary
        // node multiple times, exhausting our retry counts, and never fall
        // back to another node.
        self.choose_node_with_allowed(0)
            .or_else(|_| self.choose_node_with_allowed(self.allowed_error_count))
    }

    fn choose_node_with_allowed(
        &self,
        allowed_error_count: usize,
    ) -> Result<&Node, ConnectionError> {
        let primary_health = self.primary.is_healthy(allowed_error_count);
        if primary_health.is_healthy() {
            Ok(&self.primary)
        } else {
            let fallbacks = self
                .fallbacks
                .iter()
                .filter(|node| node.is_healthy(allowed_error_count).is_healthy())
                .collect::<Vec<_>>();

            let mut rng = rand::thread_rng();
            if let Some(node) = fallbacks.as_slice().choose(&mut rng) {
                Ok(*node)
            } else if primary_health.is_blocked() {
                Err(ConnectionError::NoHealthyFound)
            } else {
                Ok(&self.primary)
            }
        }
    }

    pub(super) fn health_report(&self) -> NodeHealthReport {
        NodeHealthReport {
            nodes: std::iter::once(self.primary.health_report(self.allowed_error_count))
                .chain(
                    self.fallbacks
                        .iter()
                        .map(|node| node.health_report(self.allowed_error_count)),
                )
                .collect(),
        }
    }

    pub(super) fn all_nodes(&self) -> AllNodes {
        AllNodes {
            primary: Some(&self.primary),
            fallbacks: self.fallbacks.iter(),
        }
    }
}

pub(crate) enum QueryResult {
    Success,
    NetworkError {
        err: QueryErrorDetails,
        action: Action,
    },
    OtherError,
}

pub(crate) struct AllNodes<'a> {
    primary: Option<&'a Node>,
    fallbacks: std::slice::Iter<'a, Node>,
}

impl<'a> Iterator for AllNodes<'a> {
    type Item = &'a Node;

    fn next(&mut self) -> Option<Self::Item> {
        match self.primary.take() {
            Some(primary) => Some(primary),
            None => self.fallbacks.next(),
        }
    }
}
