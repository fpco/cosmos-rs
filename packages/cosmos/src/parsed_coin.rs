use std::str::FromStr;

use crate::{error::ParsedCoinError, Coin};

/// Allows for parsing of a coin, it provides conversions to different coin types.
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct ParsedCoin {
    denom: String,
    amount: u128,
}

impl From<ParsedCoin> for Coin {
    fn from(ParsedCoin { denom, amount }: ParsedCoin) -> Self {
        Coin {
            denom,
            amount: amount.to_string(),
        }
    }
}

impl From<ParsedCoin> for cosmwasm_std::Coin {
    fn from(ParsedCoin { denom, amount }: ParsedCoin) -> Self {
        Self {
            denom,
            amount: amount.into(),
        }
    }
}

impl From<&ParsedCoin> for Coin {
    fn from(ParsedCoin { denom, amount }: &ParsedCoin) -> Self {
        Coin {
            denom: denom.to_owned(),
            amount: amount.to_string(),
        }
    }
}

impl From<&ParsedCoin> for cosmwasm_std::Coin {
    fn from(ParsedCoin { denom, amount }: &ParsedCoin) -> Self {
        Self {
            denom: denom.to_owned(),
            amount: (*amount).into(),
        }
    }
}

impl FromStr for ParsedCoin {
    type Err = ParsedCoinError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(ParsedCoinError::EmptyInput);
        }

        let denom_first_index = s
            .char_indices()
            .find(|(_, char)| !char.is_ascii_digit())
            .map(|(index, _)| index);

        match denom_first_index {
            None => Err(ParsedCoinError::NoDenomFound {
                input: s.to_owned(),
            }),
            Some(0) => Err(ParsedCoinError::NoAmountFound {
                input: s.to_owned(),
            }),
            Some(denom_first_index) => {
                let (amount, denom) = s.split_at(denom_first_index);

                for char in denom.chars() {
                    if !char.is_ascii_alphanumeric() && char != '/' && char != '-' {
                        return Err(ParsedCoinError::InvalidDenom {
                            input: s.to_owned(),
                        });
                    }
                }

                Ok(ParsedCoin {
                    denom: denom.to_owned(),
                    amount: amount
                        .parse()
                        .map_err(|source| ParsedCoinError::InvalidAmount {
                            input: s.to_owned(),
                            source,
                        })?,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use quickcheck::Arbitrary;

    use super::*;

    fn parse_coin(s: &str) -> Result<ParsedCoin, ParsedCoinError> {
        s.parse()
    }

    fn make_coin(amount: u128, denom: &str) -> ParsedCoin {
        ParsedCoin {
            denom: denom.to_owned(),
            amount,
        }
    }

    #[test]
    fn sanity() {
        assert_eq!(parse_coin("1ujunox").unwrap(), make_coin(1, "ujunox"));
        parse_coin("1.523ujunox").unwrap_err();
        parse_coin("foobar").unwrap_err();
        parse_coin("123ujunox!").unwrap_err();
        assert_eq!(
            parse_coin("123456uwbtc").unwrap(),
            make_coin(123456, "uwbtc")
        );
        assert_eq!(
            parse_coin("123456factory/osmo12g96ahplpf78558cv5pyunus2m66guykt96lvc/lvn1").unwrap(),
            make_coin(
                123456,
                "factory/osmo12g96ahplpf78558cv5pyunus2m66guykt96lvc/lvn1"
            )
        );
        assert_eq!(
            parse_coin("123456factory/osmo12g96ahplpf78558cv5pyunus2m66guykt96lvc/LvN1").unwrap(),
            make_coin(
                123456,
                "factory/osmo12g96ahplpf78558cv5pyunus2m66guykt96lvc/LvN1"
            )
        );
    }

    #[derive(Clone, Debug)]
    struct DenomString(String);

    impl Arbitrary for DenomString {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            // See https://github.com/BurntSushi/quickcheck/issues/279
            let sizes = (3..20).collect::<Vec<_>>();
            let letters = ('a'..='z').collect::<Vec<_>>();
            DenomString(
                (1..*g.choose(&sizes).unwrap())
                    .map(|_| *g.choose(&letters).unwrap())
                    .collect(),
            )
        }
    }

    quickcheck::quickcheck! {
        fn roundtrip(amount: u128, denom: DenomString) -> bool {
            let denom = denom.0;
            let expected = make_coin(amount, &denom);
            let actual = parse_coin(&format!("{amount}{denom}")).unwrap();
            assert_eq!(expected, actual);
            true
        }
    }
}
