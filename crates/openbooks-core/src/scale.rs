use crate::Refusal;

/// An exchange rate in fixed point with eight places, above zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rate(i64);

impl Rate {
    /// The rate of a currency to itself.
    pub const ONE: Self = Self(Self::SCALE);
    /// The fixed-point scale: one is this many units.
    pub const SCALE: i64 = 100_000_000;

    pub fn from_e8(units: i64) -> Result<Self, Refusal> {
        if units > 0 {
            Ok(Self(units))
        } else {
            Err(Refusal::BadRate)
        }
    }

    pub fn e8(self) -> i64 {
        self.0
    }
}

/// A hundredth of a percent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BasisPoints(pub u32);

impl BasisPoints {
    /// The scale: a whole is this many basis points.
    pub const WHOLE: u32 = 10_000;
}

/// A count of units in fixed point with four places.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Quantity(pub i64);

impl Quantity {
    /// The fixed-point scale: one unit is this many.
    pub const SCALE: i64 = 10_000;
}

pub(crate) fn div_round(numerator: i128, denominator: i128) -> Result<i64, Refusal> {
    let quotient = numerator.checked_div(denominator).ok_or(Refusal::Overflow)?;
    let remainder = numerator.checked_rem(denominator).ok_or(Refusal::Overflow)?;
    let away = remainder.unsigned_abs().saturating_mul(2) >= denominator.unsigned_abs();
    let rounded = if away {
        quotient + numerator.signum() * denominator.signum()
    } else {
        quotient
    };
    i64::try_from(rounded).map_err(|_| Refusal::Overflow)
}
