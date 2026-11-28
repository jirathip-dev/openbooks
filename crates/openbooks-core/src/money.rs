use std::cmp::Reverse;
use std::fmt;

use crate::scale::div_round;
use crate::{BasisPoints, Quantity, Rate, Refusal};

/// A currency by its three-letter code and the number of its minor places.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Currency {
    code: [u8; 3],
    places: u8,
}

impl Currency {
    pub fn new(code: &str, places: u8) -> Result<Self, Refusal> {
        let code: [u8; 3] = code.as_bytes().try_into().map_err(|_| Refusal::BadCurrency)?;
        if code.iter().all(u8::is_ascii_uppercase) && places <= 4 {
            Ok(Self { code, places })
        } else {
            Err(Refusal::BadCurrency)
        }
    }

    pub fn code(&self) -> &str {
        std::str::from_utf8(&self.code).unwrap_or_default()
    }

    pub fn places(self) -> u8 {
        self.places
    }

    fn minor_per_major(self) -> i128 {
        10_i128.pow(u32::from(self.places))
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// An amount in whole minor units of one currency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Money {
    minor: i64,
    currency: Currency,
}

impl Money {
    pub fn new(minor: i64, currency: Currency) -> Self {
        Self { minor, currency }
    }

    pub fn minor(self) -> i64 {
        self.minor
    }

    pub fn currency(self) -> Currency {
        self.currency
    }

    pub fn checked_add(self, other: Self) -> Result<Self, Refusal> {
        let minor = self
            .same(other)?
            .minor
            .checked_add(other.minor)
            .ok_or(Refusal::Overflow)?;
        Ok(Self { minor, ..self })
    }

    pub fn checked_sub(self, other: Self) -> Result<Self, Refusal> {
        let minor = self
            .same(other)?
            .minor
            .checked_sub(other.minor)
            .ok_or(Refusal::Overflow)?;
        Ok(Self { minor, ..self })
    }

    pub fn checked_neg(self) -> Result<Self, Refusal> {
        let minor = self.minor.checked_neg().ok_or(Refusal::Overflow)?;
        Ok(Self { minor, ..self })
    }

    /// This amount times a share in basis points, rounded half away from zero.
    pub fn share(self, share: BasisPoints) -> Result<Self, Refusal> {
        self.scaled(i128::from(share.0), i128::from(BasisPoints::WHOLE))
    }

    /// This amount as a unit price times a quantity, rounded half away from zero.
    pub fn times(self, quantity: Quantity) -> Result<Self, Refusal> {
        self.scaled(i128::from(quantity.0), i128::from(Quantity::SCALE))
    }

    /// This amount in another currency at a rate, rounded half away from zero.
    pub fn convert(self, rate: Rate, to: Currency) -> Result<Self, Refusal> {
        let numerator = i128::from(self.minor)
            .checked_mul(i128::from(rate.e8()))
            .and_then(|n| n.checked_mul(to.minor_per_major()))
            .ok_or(Refusal::Overflow)?;
        let denominator = i128::from(Rate::SCALE) * self.currency.minor_per_major();
        Ok(Self {
            minor: div_round(numerator, denominator)?,
            currency: to,
        })
    }

    /// This amount split across lines by weight, the parts summing to it to the last minor unit.
    pub fn allocate(self, weights: &[u64]) -> Result<Vec<Self>, Refusal> {
        let total: u128 = weights.iter().map(|w| u128::from(*w)).sum();
        let total = i128::try_from(total).map_err(|_| Refusal::Overflow)?;
        if total == 0 {
            return Err(Refusal::NoWeights);
        }
        let amount = i128::from(self.minor);
        let mut parts = Vec::with_capacity(weights.len());
        let mut remainders = Vec::with_capacity(weights.len());
        for (line, weight) in weights.iter().enumerate() {
            let exact = amount.checked_mul(i128::from(*weight)).ok_or(Refusal::Overflow)?;
            parts.push(exact / total);
            remainders.push((Reverse((exact % total).unsigned_abs()), line));
        }
        let left_over = amount - parts.iter().sum::<i128>();
        remainders.sort_unstable();
        for (_, line) in remainders
            .iter()
            .take(usize::try_from(left_over.unsigned_abs()).unwrap_or(usize::MAX))
        {
            if let Some(part) = parts.get_mut(*line) {
                *part += left_over.signum();
            }
        }
        parts
            .into_iter()
            .map(|minor| {
                Ok(Self {
                    minor: i64::try_from(minor).map_err(|_| Refusal::Overflow)?,
                    ..self
                })
            })
            .collect()
    }

    fn scaled(self, by: i128, scale: i128) -> Result<Self, Refusal> {
        Ok(Self {
            minor: div_round(i128::from(self.minor) * by, scale)?,
            ..self
        })
    }

    fn same(self, other: Self) -> Result<Self, Refusal> {
        if self.currency == other.currency {
            Ok(self)
        } else {
            Err(Refusal::CurrencyMismatch {
                left: self.currency,
                right: other.currency,
            })
        }
    }
}

/// An amount in its own currency with its value in the base currency of the books.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Valued {
    amount: Money,
    base: Money,
}

impl Valued {
    pub fn at(amount: Money, rate: Rate, base: Currency) -> Result<Self, Refusal> {
        if amount.currency == base && rate != Rate::ONE {
            return Err(Refusal::BadRate);
        }
        Ok(Self {
            amount,
            base: amount.convert(rate, base)?,
        })
    }

    pub fn amount(self) -> Money {
        self.amount
    }

    pub fn base(self) -> Money {
        self.base
    }
}
