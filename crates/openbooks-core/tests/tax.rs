use openbooks_core::{Currency, Date, Document, Line, Money, NoTax, Party, Rate, Refusal, TaxPack};

fn document(codes: &[&str]) -> Result<Document, Refusal> {
    let xts = Currency::new("XTS", 2)?;
    Ok(Document {
        lines: vec![Line {
            amount: Money::new(10_000, xts),
            codes: codes.iter().map(|c| (*c).to_owned()).collect(),
        }],
        party: Party {
            name: "Nolla".to_owned(),
            tax_id: None,
        },
        date: Date::new(2026, 11, 26)?,
        currency: xts,
        rate: Rate::ONE,
    })
}

#[test]
fn a_date_is_a_day_its_month_has() -> Result<(), Refusal> {
    assert_eq!(Date::new(2024, 2, 29)?.day(), 29);
    assert_eq!(Date::new(2000, 2, 29)?.month().year(), 2000);
    for (year, month, day) in [
        (2026, 2, 29),
        (1900, 2, 29),
        (2026, 4, 31),
        (2026, 13, 1),
        (2026, 1, 0),
        (0, 1, 1),
    ] {
        assert_eq!(
            Date::new(year, month, day),
            Err(Refusal::BadDate),
            "{year}-{month}-{day}"
        );
    }
    Ok(())
}

#[test]
fn the_no_tax_pack_has_no_codes_makes_no_tax_lines_and_refuses_any_code() -> Result<(), Refusal> {
    let plain = document(&[])?;
    assert!(NoTax.added().is_empty() && NoTax.withheld().is_empty() && NoTax.note_reasons().is_empty());
    assert_eq!(NoTax.document_tax(&plain)?, []);
    assert_eq!(NoTax.settlement_tax(&plain, &[])?, []);
    assert_eq!(NoTax.check_party(&plain.party), Ok(()));
    assert_eq!(NoTax.document_tax(&document(&["T7"])?), Err(Refusal::UnknownTaxCode));
    assert_eq!(NoTax.settlement_tax(&plain, &["W3"]), Err(Refusal::UnknownTaxCode));
    Ok(())
}
