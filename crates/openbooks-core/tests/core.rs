use openbooks_core::{BasisPoints, BookKind, Currency, Money, Month, OrgId, Quantity, Rate, Refusal, UserId, Valued};

const CASES: usize = 20_000;

struct Gen(u64);

impl Gen {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    fn amount(&mut self) -> i64 {
        let bits = self.below(48) + 1;
        let magnitude = i64::try_from(self.below(1 << bits)).unwrap_or(0);
        if self.below(2) == 0 { magnitude } else { -magnitude }
    }
}

fn cur(code: &str, places: u8) -> Result<Currency, Refusal> {
    Currency::new(code, places)
}

#[test]
fn an_allocation_sums_to_the_total_and_each_part_is_within_a_minor_unit_of_its_share() -> Result<(), Refusal> {
    let xts = cur("XTS", 2)?;
    let mut generator = Gen(4);
    for _ in 0..CASES {
        let total = generator.amount();
        let lines = generator.below(12) + 1;
        let weights: Vec<u64> = (0..lines)
            .map(|_| generator.below(4) * generator.below(1_000_000))
            .collect();
        let weight: i128 = weights.iter().map(|w| i128::from(*w)).sum();
        if weight == 0 {
            assert_eq!(Money::new(total, xts).allocate(&weights), Err(Refusal::NoWeights));
            continue;
        }
        let parts = Money::new(total, xts).allocate(&weights)?;
        assert_eq!(parts.len(), weights.len());
        assert_eq!(
            parts.iter().map(|p| i128::from(p.minor())).sum::<i128>(),
            i128::from(total)
        );
        for (part, w) in parts.iter().zip(&weights) {
            let gap = i128::from(part.minor()) * weight - i128::from(total) * i128::from(*w);
            assert!(gap.abs() < weight, "{total} by {weights:?} gave {parts:?}");
            if *w == 0 {
                assert_eq!(part.minor(), 0);
            }
            assert_eq!(part.currency(), xts);
        }
    }
    Ok(())
}

#[test]
fn an_allocation_gives_the_left_over_unit_to_the_largest_remainder_then_the_first_line() -> Result<(), Refusal> {
    let xts = cur("XTS", 2)?;
    let minors = |total: i64, weights: &[u64]| -> Result<Vec<i64>, Refusal> {
        Ok(Money::new(total, xts)
            .allocate(weights)?
            .iter()
            .map(|m| m.minor())
            .collect())
    };
    assert_eq!(minors(100, &[1, 1, 1])?, vec![34, 33, 33]);
    assert_eq!(minors(-100, &[1, 1, 1])?, vec![-34, -33, -33]);
    assert_eq!(minors(5, &[1, 2])?, vec![2, 3]);
    assert_eq!(minors(1, &[0, 1, 1])?, vec![0, 1, 0]);
    assert_eq!(Money::new(1, xts).allocate(&[]), Err(Refusal::NoWeights));
    Ok(())
}

#[test]
fn a_share_and_a_quantity_round_half_away_from_zero() -> Result<(), Refusal> {
    let xts = cur("XTS", 2)?;
    let mut generator = Gen(7);
    for _ in 0..CASES {
        let minor = generator.amount();
        let points = u32::try_from(generator.below(30_000)).unwrap_or(0);
        let share = Money::new(minor, xts).share(BasisPoints(points))?.minor();
        let gap = i128::from(share) * 10_000 - i128::from(minor) * i128::from(points);
        assert!(gap.abs() <= 5_000 && (gap.abs() < 5_000 || gap.signum() == i128::from(minor).signum()));
        assert_eq!(
            Money::new(minor, xts)
                .checked_neg()?
                .share(BasisPoints(points))?
                .minor(),
            -share
        );
        assert_eq!(
            Money::new(minor, xts).share(BasisPoints(BasisPoints::WHOLE))?.minor(),
            minor
        );
        let count = generator.amount() % 10_000_000;
        let price = generator.amount() % 1_000_000_000;
        let line = Money::new(price, xts).times(Quantity(count))?.minor();
        let gap = i128::from(line) * 10_000 - i128::from(price) * i128::from(count);
        assert!(gap.abs() <= 5_000);
        assert_eq!(Money::new(price, xts).times(Quantity(Quantity::SCALE))?.minor(), price);
    }
    assert_eq!(Money::new(5, xts).share(BasisPoints(1_000))?.minor(), 1);
    assert_eq!(Money::new(-5, xts).share(BasisPoints(1_000))?.minor(), -1);
    assert_eq!(Money::new(4, xts).share(BasisPoints(1_000))?.minor(), 0);
    Ok(())
}

#[test]
fn a_conversion_rounds_to_the_nearest_minor_unit_of_the_target() -> Result<(), Refusal> {
    let currencies = [cur("XTS", 2)?, cur("XAA", 0)?, cur("XBB", 3)?, cur("XCC", 4)?];
    let mut generator = Gen(11);
    for _ in 0..CASES {
        let from = *currencies
            .get(usize::try_from(generator.below(4)).unwrap_or(0))
            .ok_or(Refusal::BadCurrency)?;
        let to = *currencies
            .get(usize::try_from(generator.below(4)).unwrap_or(0))
            .ok_or(Refusal::BadCurrency)?;
        let minor = generator.amount();
        let rate = Rate::from_e8(i64::try_from(generator.below(1 << 40) + 1).unwrap_or(1))?;
        let scale = 100_000_000 * 10_i128.pow(u32::from(from.places()));
        let exact = i128::from(minor) * i128::from(rate.e8()) * 10_i128.pow(u32::from(to.places()));
        let Ok(out) = Money::new(minor, from).convert(rate, to) else {
            assert!(i64::try_from(exact / scale).is_err());
            continue;
        };
        assert_eq!(out.currency(), to);
        assert!((i128::from(out.minor()) * scale - exact).abs() * 2 <= scale);
        assert_eq!(
            Money::new(minor, from).checked_neg()?.convert(rate, to)?.minor(),
            -out.minor()
        );
        assert_eq!(Money::new(minor, from).convert(Rate::ONE, from)?.minor(), minor);
    }
    let xts = cur("XTS", 2)?;
    let xaa = cur("XAA", 0)?;
    assert_eq!(Money::new(150, xts).convert(Rate::ONE, xaa)?.minor(), 2);
    assert_eq!(Money::new(149, xts).convert(Rate::ONE, xaa)?.minor(), 1);
    assert_eq!(
        Money::new(1, xaa).convert(Rate::from_e8(3_512_345_678)?, xts)?.minor(),
        3_512
    );
    assert_eq!(
        Money::new(i64::MAX, xaa).convert(Rate::from_e8(i64::MAX)?, xts),
        Err(Refusal::Overflow)
    );
    Ok(())
}

#[test]
fn money_of_two_currencies_does_not_mix() -> Result<(), Refusal> {
    let (xts, xaa) = (cur("XTS", 2)?, cur("XAA", 0)?);
    let mismatch = Err(Refusal::CurrencyMismatch { left: xts, right: xaa });
    assert_eq!(Money::new(1, xts).checked_add(Money::new(1, xaa)), mismatch);
    assert_eq!(Money::new(1, xts).checked_sub(Money::new(1, xaa)), mismatch);
    assert_eq!(
        Money::new(5, xts).checked_add(Money::new(-7, xts))?,
        Money::new(-2, xts)
    );
    assert_eq!(Money::new(5, xts).checked_sub(Money::new(7, xts))?, Money::new(-2, xts));
    assert_eq!(
        Money::new(i64::MAX, xts).checked_add(Money::new(1, xts)),
        Err(Refusal::Overflow)
    );
    assert_eq!(Money::new(i64::MIN, xts).checked_neg(), Err(Refusal::Overflow));
    Ok(())
}

#[test]
fn a_valued_amount_carries_its_base_value_and_refuses_a_rate_to_itself() -> Result<(), Refusal> {
    let (xts, xaa) = (cur("XTS", 2)?, cur("XAA", 0)?);
    let rate = Rate::from_e8(250_000_000)?;
    let valued = Valued::at(Money::new(1_001, xts), rate, xaa)?;
    assert_eq!(
        (valued.amount(), valued.base()),
        (Money::new(1_001, xts), Money::new(25, xaa))
    );
    let home = Valued::at(Money::new(7, xts), Rate::ONE, xts)?;
    assert_eq!(home.base(), home.amount());
    assert_eq!(Valued::at(Money::new(7, xts), rate, xts), Err(Refusal::BadRate));
    Ok(())
}

#[test]
fn values_at_the_edge_are_refused_with_their_reason() -> Result<(), Refusal> {
    for bad in ["", "XT", "XTSS", "xts", "X1S"] {
        assert_eq!(Currency::new(bad, 2), Err(Refusal::BadCurrency));
    }
    assert_eq!(Currency::new("XTS", 5), Err(Refusal::BadCurrency));
    assert_eq!(cur("XTS", 2)?.to_string(), "XTS");
    assert_eq!(Rate::from_e8(0), Err(Refusal::BadRate));
    assert_eq!(Rate::from_e8(-1), Err(Refusal::BadRate));
    assert_eq!(
        (OrgId::new(0), UserId::new(-3)),
        (Err(Refusal::BadId), Err(Refusal::BadId))
    );
    assert_eq!((OrgId::new(9)?.get(), UserId::new(4)?.get()), (9, 4));
    for (year, month) in [(2026, 0), (2026, 13), (0, 1), (10_000, 1)] {
        assert_eq!(Month::new(year, month), Err(Refusal::BadMonth));
    }
    let november = Month::new(2026, 11)?;
    assert!(november < Month::new(2026, 12)? && Month::new(2025, 12)? < november);
    assert_eq!((november.year(), november.month()), (2026, 11));
    for kind in BookKind::ALL {
        assert_eq!(BookKind::parse(kind.as_str()), Ok(kind));
    }
    assert_eq!(BookKind::parse("receivables"), Err(Refusal::BadBookKind));
    Ok(())
}
