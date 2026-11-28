use crate::{Currency, Date, Money, Rate, Refusal};

/// When a tax falls due: when the document is issued, or when it is settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Due {
    Document,
    Settlement,
}

/// A tax a pack knows, by its code, and when it falls due.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaxCode {
    pub code: &'static str,
    pub due: Due,
}

/// A party to a document as the pack sees it: its name and its tax id, if it has one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Party {
    pub name: String,
    pub tax_id: Option<String>,
}

/// A document line: its amount before tax and the tax codes it carries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Line {
    pub amount: Money,
    pub codes: Vec<String>,
}

/// What a pack reckons tax on: a document's lines, or the part of them a settlement pays, with its party, date and rate.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Document {
    pub lines: Vec<Line>,
    pub party: Party,
    pub date: Date,
    pub currency: Currency,
    pub rate: Rate,
}

impl Document {
    /// The sum of the lines that carry `code`, or of every line with no code given.
    pub fn taxable(&self, code: Option<&str>) -> Result<Money, Refusal> {
        let carries = |line: &&Line| code.is_none_or(|c| line.codes.iter().any(|l| l == c));
        self.lines
            .iter()
            .filter(carries)
            .try_fold(Money::new(0, self.currency), |sum, line| sum.checked_add(line.amount))
    }

    /// Refuses a line code that is not one of `known`.
    pub fn only(&self, known: &[TaxCode]) -> Result<(), Refusal> {
        let mut codes = self.lines.iter().flat_map(|line| &line.codes);
        if codes.all(|c| known.iter().any(|k| k.code == c)) {
            Ok(())
        } else {
            Err(Refusal::UnknownTaxCode)
        }
    }
}

/// One tax a document or settlement carries: its code, what it was reckoned on and the tax.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaxLine {
    pub code: &'static str,
    pub taxable: Money,
    pub tax: Money,
}

/// A reason a pack offers for a credit or debit note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NoteReason {
    pub code: &'static str,
    pub label: &'static str,
}

/// A country's tax rules over plain values: its codes, the tax lines they make, its party check and note reasons.
pub trait TaxPack: Send + Sync {
    fn added(&self) -> &[TaxCode];

    fn withheld(&self) -> &[TaxCode];

    fn document_tax(&self, document: &Document) -> Result<Vec<TaxLine>, Refusal>;

    fn settlement_tax(&self, settled: &Document, withheld: &[&str]) -> Result<Vec<TaxLine>, Refusal>;

    fn check_party(&self, party: &Party) -> Result<(), Refusal>;

    fn note_reasons(&self) -> &[NoteReason];
}

/// The rules of a company that charges and withholds no tax.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTax;

impl TaxPack for NoTax {
    fn added(&self) -> &[TaxCode] {
        &[]
    }

    fn withheld(&self) -> &[TaxCode] {
        &[]
    }

    fn document_tax(&self, document: &Document) -> Result<Vec<TaxLine>, Refusal> {
        document.only(&[]).map(|()| Vec::new())
    }

    fn settlement_tax(&self, settled: &Document, withheld: &[&str]) -> Result<Vec<TaxLine>, Refusal> {
        if withheld.is_empty() {
            self.document_tax(settled)
        } else {
            Err(Refusal::UnknownTaxCode)
        }
    }

    fn check_party(&self, _party: &Party) -> Result<(), Refusal> {
        Ok(())
    }

    fn note_reasons(&self) -> &[NoteReason] {
        &[]
    }
}
