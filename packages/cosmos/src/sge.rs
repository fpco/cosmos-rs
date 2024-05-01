//! SGE-specific functionality.

use crate::{
    error::{Action, ChainParseError},
    Cosmos, Error,
};
use cosmwasm_std::Decimal;

pub(crate) mod mint;

impl Cosmos {
    /// Get the SGE special inflation information
    ///
    /// Note that this query will fail if called on chains besides SGE.
    pub async fn sge_inflation(&self) -> Result<DecimalQueryInflationResponse, Error> {
        self.perform_query(mint::QueryInflationRequest {}, Action::SgeInflation, true)
            .await?
            .into_inner()
            .try_into()
    }
}

/// Copy of the SGE QueryInflationResponse but using Decimal.
/// The inflation number in grpc is given as the bytes Vec<u8> of a numeric string,
/// that has to be offset 18 decimal places to get the unit amount.
/// We parse it into a Decimal to make it more usable.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct DecimalQueryInflationResponse {
    inflation: Decimal,
}

impl TryFrom<mint::QueryInflationResponse> for DecimalQueryInflationResponse {
    type Error = Error;

    fn try_from(value: mint::QueryInflationResponse) -> Result<Self, Self::Error> {
        let stringy = std::str::from_utf8(&value.inflation).map_err(make_err)?;
        let parsed = stringy.parse::<u128>().map_err(make_err)?;
        let inflation = Decimal::from_atomics(parsed, 18).map_err(make_err)?;
        Ok(Self { inflation })
    }
}

fn make_err(err: impl std::fmt::Display) -> Error {
    Error::ChainParse {
        source: Box::new(ChainParseError::WeiAmount {
            err: err.to_string(),
        }),
        action: Action::SgeInflation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inflation_of_one_point() {
        let source = mint::QueryInflationResponse {
            inflation: b"10000000000000000".to_vec(),
        };
        assert_eq!(
            DecimalQueryInflationResponse::try_from(source)
                .unwrap()
                .inflation,
            Decimal::percent(1)
        );
    }

    #[test]
    fn api_changes_will_break() {
        let source = mint::QueryInflationResponse {
            inflation: b"0.01".to_vec(),
        };
        let error = DecimalQueryInflationResponse::try_from(source)
            .unwrap_err()
            .to_string();

        assert!(error.contains("invalid digit found in string"));
    }
}
