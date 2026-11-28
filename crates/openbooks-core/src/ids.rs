use crate::Refusal;

macro_rules! id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(i64);

        impl $name {
            pub fn new(value: i64) -> Result<Self, Refusal> {
                if value > 0 {
                    Ok(Self(value))
                } else {
                    Err(Refusal::BadId)
                }
            }

            pub fn get(self) -> i64 {
                self.0
            }
        }
    };
}

id!(OrgId, "An organisation that keeps a set of books.");
id!(UserId, "A person who signs in.");
id!(ProjectId, "A project whose costs and income are tracked apart.");

/// Which book an entry belongs to: the general ledger or a sub-ledger over it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BookKind {
    General,
    Receivables,
    Payables,
    Projects,
}

/// A calendar month of a year, the unit of a period.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Month {
    year: u16,
    month: u8,
}

impl Month {
    pub fn new(year: u16, month: u8) -> Result<Self, Refusal> {
        if (1..=9999).contains(&year) && (1..=12).contains(&month) {
            Ok(Self { year, month })
        } else {
            Err(Refusal::BadMonth)
        }
    }

    pub fn year(self) -> u16 {
        self.year
    }

    pub fn month(self) -> u8 {
        self.month
    }
}
