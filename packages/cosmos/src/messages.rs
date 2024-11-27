//! Message types provided directly by this library (instead of from the protobufs).

use std::{fmt::Display, path::PathBuf};

use chrono::{DateTime, Utc};
use cosmos_sdk_proto::{
    cosmos::{
        authz::v1beta1::{GenericAuthorization, Grant, MsgExec, MsgGrant},
        bank::v1beta1::MsgSend,
        base::v1beta1::Coin,
    },
    cosmwasm::wasm::v1::{
        MsgExecuteContract, MsgInstantiateContract, MsgMigrateContract, MsgStoreCode,
        MsgUpdateAdmin,
    },
    traits::Message,
};

use crate::{error::StringOrBytes, Address, HasAddress, TxMessage};

/// A local version of [MsgExec] with extra information for nice error messages.
pub struct MsgExecHelper {
    /// See [MsgExec::grantee]
    pub grantee: Address,
    /// Uses [TxMessage] to provide more introspection on what we're doing
    pub msgs: Vec<TxMessage>,
}

impl From<MsgExecHelper> for TxMessage {
    fn from(MsgExecHelper { grantee, msgs }: MsgExecHelper) -> Self {
        let mut raw_msgs = vec![];
        let mut descs = vec![];

        for msg in msgs {
            let (msg, desc) = msg.into_protobuf();
            raw_msgs.push(msg);
            descs.push(desc);
        }
        TxMessage::new(
            "/cosmos.authz.v1beta1.MsgExec",
            MsgExec {
                grantee: grantee.get_address_string(),
                msgs: raw_msgs,
            }
            .encode_to_vec(),
            format!("Grantee {grantee} executing: {descs:?}"),
        )
    }
}

/// A message for granting authorization to another address.
pub struct MsgGrantHelper {
    /// Address granting permissions
    pub granter: Address,
    /// Address receiving permissions
    pub grantee: Address,
    /// Which features are being authorized
    pub authorization: String,
    /// When the authorization expires
    pub expiration: Option<DateTime<Utc>>,
}

impl From<MsgGrantHelper> for TxMessage {
    fn from(
        MsgGrantHelper {
            granter,
            grantee,
            authorization,
            expiration,
        }: MsgGrantHelper,
    ) -> Self {
        let desc = format!(
            "{granter} grants {grantee} authorization for {authorization} until {expiration:?}"
        );
        let authorization = GenericAuthorization { msg: authorization };
        let authorization = cosmos_sdk_proto::Any {
            type_url: "/cosmos.authz.v1beta1.GenericAuthorization".to_owned(),
            value: authorization.encode_to_vec(),
        };
        let msg_grant = MsgGrant {
            granter: granter.get_address_string(),
            grantee: grantee.get_address_string(),
            grant: Some(Grant {
                authorization: Some(authorization),
                expiration: expiration.map(datetime_to_timestamp),
            }),
        };
        TxMessage::new(
            "/cosmos.authz.v1beta1.GenericAuthorization",
            msg_grant.encode_to_vec(),
            desc,
        )
    }
}

fn datetime_to_timestamp(x: DateTime<Utc>) -> cosmos_sdk_proto::Timestamp {
    cosmos_sdk_proto::Timestamp {
        seconds: x.timestamp(),
        nanos: x
            .timestamp_subsec_nanos()
            .try_into()
            .expect("DateTime<Utc>'s nanos is too large"),
    }
}

/// A helper for [MsgStoreCode] that provides source path information.
pub struct MsgStoreCodeHelper {
    /// See [MsgStoreCode::sender]
    pub sender: Address,
    /// See [MsgStoreCode::wasm_byte_code]
    pub wasm_byte_code: Vec<u8>,
    /// File path this came from, if known
    pub source: Option<PathBuf>,
}

impl From<MsgStoreCodeHelper> for TxMessage {
    fn from(
        MsgStoreCodeHelper {
            sender,
            wasm_byte_code,
            source,
        }: MsgStoreCodeHelper,
    ) -> Self {
        TxMessage::new(
            "/cosmwasm.wasm.v1.MsgStoreCode",
            MsgStoreCode {
                sender: sender.get_address_string(),
                wasm_byte_code,
                instantiate_permission: None,
            }
            .encode_to_vec(),
            match source {
                Some(path) => format!("Storing WASM code loaded from {}", path.display()),
                None => "Storing WASM code from unknown location".to_owned(),
            },
        )
    }
}

impl From<MsgInstantiateContract> for TxMessage {
    fn from(msg: MsgInstantiateContract) -> Self {
        TxMessage::new(
            "/cosmwasm.wasm.v1.MsgInstantiateContract",
            msg.encode_to_vec(),
            format!(
                "{} instantiating code ID {} with label {} and message: {}",
                msg.sender,
                msg.code_id,
                msg.label,
                StringOrBytes(msg.msg)
            ),
        )
    }
}

impl From<MsgMigrateContract> for TxMessage {
    fn from(msg: MsgMigrateContract) -> Self {
        TxMessage::new(
            "/cosmwasm.wasm.v1.MsgMigrateContract",
            msg.encode_to_vec(),
            format!(
                "{} migrating contract {} to code ID {} with message: {}",
                msg.sender,
                msg.contract,
                msg.code_id,
                StringOrBytes(msg.msg)
            ),
        )
    }
}

impl From<MsgExecuteContract> for TxMessage {
    fn from(msg: MsgExecuteContract) -> Self {
        TxMessage::new(
            "/cosmwasm.wasm.v1.MsgExecuteContract",
            msg.encode_to_vec(),
            format!(
                "{} executing contract {} with message: {}",
                msg.sender,
                msg.contract,
                StringOrBytes(msg.msg)
            ),
        )
    }
}

impl From<MsgUpdateAdmin> for TxMessage {
    fn from(msg: MsgUpdateAdmin) -> Self {
        TxMessage::new(
            "/cosmwasm.wasm.v1.MsgUpdateAdmin",
            msg.encode_to_vec(),
            format!(
                "{} updating admin on {} to {}",
                msg.sender, msg.contract, msg.new_admin
            ),
        )
    }
}

impl From<MsgSend> for TxMessage {
    fn from(msg: MsgSend) -> Self {
        // Very hacky approach to sending the alternative MsgSend
        // on THORChain. For simplicity, we simply look for an appropriate
        // HRP prefix on the destination address.
        if msg.to_address.starts_with("sthor1") {
            #[derive(::prost::Message)]
            struct ThorMsgSend {
                /// Source of funds
                #[prost(bytes, tag = "1")]
                from_address: Vec<u8>,
                /// Destination of funds
                #[prost(bytes, tag = "2")]
                to_address: Vec<u8>,
                /// Funds to be sent
                #[prost(message, repeated, tag = "3")]
                amount: ::prost::alloc::vec::Vec<cosmos_sdk_proto::cosmos::base::v1beta1::Coin>,
            }
            let description = format!(
                "{} sending (via THORChain message) {} to {}",
                msg.from_address,
                PrettyCoins(msg.amount.as_slice()),
                msg.to_address,
            );
            fn get_acc_address(s: &str) -> Vec<u8> {
                // This code will panic if an invalid address is provided.
                // We could fix that, but given how corner a case this is,
                // I'm not interested in changing the overall API to accommodate
                // it.
                bech32::decode(s)
                    .expect("Invalid THORChain address provided for MsgSend")
                    .1
            }
            let thormsg = ThorMsgSend {
                from_address: get_acc_address(&msg.from_address),
                to_address: get_acc_address(&msg.to_address),
                amount: msg.amount,
            };

            TxMessage::new("/types.MsgSend", thormsg.encode_to_vec(), description)
        } else {
            TxMessage::new(
                "/cosmos.bank.v1beta1.MsgSend",
                msg.encode_to_vec(),
                format!(
                    "{} sending {} to {}",
                    msg.from_address,
                    PrettyCoins(msg.amount.as_slice()),
                    msg.to_address,
                ),
            )
        }
    }
}

struct PrettyCoins<'a>(&'a [Coin]);
impl Display for PrettyCoins<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        for (idx, Coin { denom, amount }) in self.0.iter().enumerate() {
            if idx > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{amount}{denom}")?;
        }
        Ok(())
    }
}
