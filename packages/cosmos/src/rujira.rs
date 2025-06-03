use prost::Message;
use tonic::{async_trait, GrpcMethod};

use crate::{
    client::{node::Node, query::GrpcRequest},
    error::Action,
    Address, Cosmos, TxMessage,
};

impl Cosmos {
    /// Query information on a Rujira pool
    pub async fn rujira_pool(
        &self,
        asset: impl Into<String>,
    ) -> Result<QueryPoolResponse, crate::Error> {
        Ok(self
            .perform_query(
                QueryPoolRequest {
                    asset: asset.into(),
                    height: "".to_owned(),
                },
                Action::GetLatestBlock,
            )
            .run()
            .await?
            .into_inner())
    }

    /// Query information on available Rujira pools
    pub async fn rujira_pools(&self) -> Result<QueryPoolsResponse, crate::Error> {
        Ok(self
            .perform_query(
                QueryPoolsRequest {
                    height: "".to_owned(),
                },
                Action::GetLatestBlock,
            )
            .run()
            .await?
            .into_inner())
    }
}

pub(crate) struct RujiraQueryClient<T> {
    inner: tonic::client::Grpc<T>,
}
impl<T> RujiraQueryClient<T>
where
    T: tonic::client::GrpcService<tonic::body::BoxBody>,
    T::Error: Into<tonic::codegen::StdError>,
    T::ResponseBody: tonic::codegen::Body<Data = tonic::codegen::Bytes> + Send + 'static,
    <T::ResponseBody as tonic::codegen::Body>::Error: Into<tonic::codegen::StdError> + Send,
{
    pub(crate) fn new(inner: T) -> Self {
        let inner = tonic::client::Grpc::new(inner);
        Self { inner }
    }

    async fn pool(
        &mut self,
        request: impl tonic::IntoRequest<QueryPoolRequest>,
    ) -> Result<tonic::Response<QueryPoolResponse>, tonic::Status> {
        self.inner.ready().await.map_err(|e| {
            tonic::Status::new(
                tonic::Code::Unknown,
                format!("Service was not ready: {}", e.into()),
            )
        })?;
        let codec = tonic::codec::ProstCodec::default();
        let path = http::uri::PathAndQuery::from_static("/types.Query/Pool");
        let mut req = request.into_request();
        req.extensions_mut()
            .insert(GrpcMethod::new("types.Query", "Pool"));
        self.inner.unary(req, path, codec).await
    }

    async fn pools(
        &mut self,
        request: impl tonic::IntoRequest<QueryPoolsRequest>,
    ) -> Result<tonic::Response<QueryPoolsResponse>, tonic::Status> {
        self.inner.ready().await.map_err(|e| {
            tonic::Status::new(
                tonic::Code::Unknown,
                format!("Service was not ready: {}", e.into()),
            )
        })?;
        let codec = tonic::codec::ProstCodec::default();
        let path = http::uri::PathAndQuery::from_static("/types.Query/Pools");
        let mut req = request.into_request();
        req.extensions_mut()
            .insert(GrpcMethod::new("types.Query", "Pools"));
        self.inner.unary(req, path, codec).await
    }
}

#[async_trait]
impl GrpcRequest for QueryPoolRequest {
    type Response = QueryPoolResponse;

    async fn perform(
        req: tonic::Request<Self>,
        inner: &Node,
    ) -> Result<tonic::Response<Self::Response>, tonic::Status> {
        inner.rujira_query_client().pool(req).await
    }
}

#[async_trait]
impl GrpcRequest for QueryPoolsRequest {
    type Response = QueryPoolsResponse;

    async fn perform(
        req: tonic::Request<Self>,
        inner: &Node,
    ) -> Result<tonic::Response<Self::Response>, tonic::Status> {
        inner.rujira_query_client().pools(req).await
    }
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub(crate) struct QueryPoolRequest {
    #[prost(string, tag = "1")]
    pub asset: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub height: ::prost::alloc::string::String,
}

/// Response with information on a THORChain pool.
#[derive(Clone, PartialEq, ::prost::Message)]
#[allow(missing_docs)]
pub struct QueryPoolResponse {
    #[prost(string, tag = "1")]
    pub asset: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub short_code: ::prost::alloc::string::String,
    #[prost(string, tag = "3")]
    pub status: ::prost::alloc::string::String,
    #[prost(int64, tag = "4")]
    pub decimals: i64,
    #[prost(string, tag = "5")]
    pub pending_inbound_asset: ::prost::alloc::string::String,
    #[prost(string, tag = "6")]
    pub pending_inbound_rune: ::prost::alloc::string::String,
    #[prost(string, tag = "7")]
    pub balance_asset: ::prost::alloc::string::String,
    #[prost(string, tag = "8")]
    pub balance_rune: ::prost::alloc::string::String,
    /// the USD (TOR) price of the asset in 1e8
    #[prost(string, tag = "9")]
    pub asset_tor_price: ::prost::alloc::string::String,
    /// the total pool units, this is the sum of LP and synth units
    #[prost(string, tag = "10")]
    pub pool_units: ::prost::alloc::string::String,
    /// the total pool liquidity provider units
    #[prost(string, tag = "11")]
    pub lp_units: ::prost::alloc::string::String,
    /// the total synth units in the pool
    #[prost(string, tag = "12")]
    pub synth_units: ::prost::alloc::string::String,
    /// the total supply of synths for the asset
    #[prost(string, tag = "13")]
    pub synth_supply: ::prost::alloc::string::String,
    /// the balance of L1 asset deposited into the Savers Vault
    #[prost(string, tag = "14")]
    pub savers_depth: ::prost::alloc::string::String,
    /// the number of units owned by Savers
    #[prost(string, tag = "15")]
    pub savers_units: ::prost::alloc::string::String,
    /// the filled savers capacity in basis points, 4500/10000 = 45%
    #[prost(string, tag = "16")]
    pub savers_fill_bps: ::prost::alloc::string::String,
    /// amount of remaining capacity in asset
    #[prost(string, tag = "17")]
    pub savers_capacity_remaining: ::prost::alloc::string::String,
    /// whether additional synths cannot be minted
    #[prost(bool, tag = "18")]
    pub synth_mint_paused: bool,
    /// the amount of synth supply remaining before the current max supply is reached
    #[prost(string, tag = "19")]
    pub synth_supply_remaining: ::prost::alloc::string::String,
    /// the amount of collateral collects for loans
    #[prost(string, tag = "20")]
    pub loan_collateral: ::prost::alloc::string::String,
    /// the amount of remaining collateral collects for loans
    #[prost(string, tag = "21")]
    pub loan_collateral_remaining: ::prost::alloc::string::String,
    /// the current loan collateralization ratio
    #[prost(string, tag = "22")]
    pub loan_cr: ::prost::alloc::string::String,
    /// the depth of the derived virtual pool relative to L1 pool (in basis points)
    #[prost(string, tag = "23")]
    pub derived_depth_bps: ::prost::alloc::string::String,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub(crate) struct QueryPoolsRequest {
    #[prost(string, tag = "1")]
    pub height: ::prost::alloc::string::String,
}
#[derive(Clone, PartialEq, ::prost::Message)]
#[allow(missing_docs)]
pub struct QueryPoolsResponse {
    #[prost(message, repeated, tag = "1")]
    pub pools: ::prost::alloc::vec::Vec<QueryPoolResponse>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct MsgDepositInner {
    #[prost(message, repeated, tag = "1")]
    coins: Vec<RujiraCoin>,
    #[prost(string, tag = "2")]
    memo: String,
    #[prost(bytes, tag = "3")]
    signer: Vec<u8>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct RujiraCoin {
    #[prost(message, tag = "1")]
    asset: Option<RujiraAsset>,
    #[prost(string, tag = "2")]
    amount: String,
    #[prost(int64, tag = "3")]
    decimals: i64,
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct RujiraAsset {
    #[prost(string, tag = "1")]
    chain: String,
    #[prost(string, tag = "2")]
    symbol: String,
    #[prost(string, tag = "3")]
    ticker: String,
    #[prost(bool, tag = "4")]
    synth: bool,
    #[prost(bool, tag = "5")]
    trade: bool,
    #[prost(bool, tag = "6")]
    secured: bool,
}

/// A Rujira `MsgDeposit`, used for withdrawing secured assets.
pub struct MsgDeposit {
    /// Chain to withdraw to
    pub chain: String,
    /// Symbol to withdraw
    pub symbol: String,
    /// Amount of asset to withdraw, given in 10e-8 units
    pub amount: u128,
    /// Destination to send the asset to
    pub destination: String,
    /// Account sending the funds
    pub signer: Address,
}

impl From<MsgDeposit> for TxMessage {
    fn from(
        MsgDeposit {
            chain,
            symbol,
            amount,
            destination,
            signer,
        }: MsgDeposit,
    ) -> Self {
        let description =
            format!("Withdraw secured assets: {amount}{chain}-{symbol} to {destination}");
        TxMessage::new(
            "/types.MsgDeposit",
            MsgDepositInner {
                coins: vec![RujiraCoin {
                    asset: Some(RujiraAsset {
                        chain,
                        symbol: symbol.clone(),
                        ticker: symbol,
                        synth: false,
                        trade: false,
                        secured: true,
                    }),
                    amount: amount.to_string(),
                    decimals: 0,
                }],
                memo: format!("secure-:{destination}"),
                signer: signer.raw().as_ref().to_owned(),
            }
            .encode_to_vec(),
            description,
        )
    }
}
