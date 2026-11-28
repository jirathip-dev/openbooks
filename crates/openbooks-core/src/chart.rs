use crate::Refusal;

/// What an account holds: the five classes every chart is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AccountType {
    Asset,
    Liability,
    Equity,
    Income,
    Expense,
}

impl AccountType {
    pub const ALL: [Self; 5] = [Self::Asset, Self::Liability, Self::Equity, Self::Income, Self::Expense];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Asset => "asset",
            Self::Liability => "liability",
            Self::Equity => "equity",
            Self::Income => "income",
            Self::Expense => "expense",
        }
    }

    pub fn parse(text: &str) -> Result<Self, Refusal> {
        Self::ALL
            .into_iter()
            .find(|t| t.as_str() == text)
            .ok_or(Refusal::BadAccountType)
    }
}

/// What an item is: goods kept in stock, goods not kept in stock, or a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ItemKind {
    Stock,
    NonStock,
    Service,
}

impl ItemKind {
    pub const ALL: [Self; 3] = [Self::Stock, Self::NonStock, Self::Service];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stock => "stock",
            Self::NonStock => "non_stock",
            Self::Service => "service",
        }
    }

    pub fn parse(text: &str) -> Result<Self, Refusal> {
        Self::ALL
            .into_iter()
            .find(|k| k.as_str() == text)
            .ok_or(Refusal::BadItemKind)
    }
}
