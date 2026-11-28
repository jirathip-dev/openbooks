use openbooks_core::{NoTax, TaxPack};
use openbooks_db::Migration;

use crate::{Hooks, Mount};

/// A part an account plays that a pack's postings look it up by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountRole {
    pub key: &'static str,
    pub about: &'static str,
}

/// An account of a pack's starting chart, with the role it plays, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChartAccount {
    pub code: &'static str,
    pub name: &'static str,
    pub role: Option<&'static str>,
}

/// The names a company's five books start with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookNames {
    pub general: &'static str,
    pub sales: &'static str,
    pub purchases: &'static str,
    pub receipts: &'static str,
    pub payments: &'static str,
}

impl BookNames {
    /// The names a pack that says none gets.
    pub const PLAIN: Self = Self {
        general: "General journal",
        sales: "Sales",
        purchases: "Purchases",
        receipts: "Receipts",
        payments: "Payments",
    };
}

/// A number series: the kind of document it numbers and the prefix of its numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Series {
    pub document: &'static str,
    pub prefix: &'static str,
}

/// A print or message template a pack brings, by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Template {
    pub name: &'static str,
    pub body: &'static str,
}

/// A country's tax rules and what it adds through the host's own doors; a company keeps its books under one.
pub trait Pack: Hooks {
    fn name(&self) -> &'static str;

    fn tax(&self) -> &dyn TaxPack;

    fn account_roles(&self) -> &[AccountRole] {
        &[]
    }

    fn chart(&self) -> &[ChartAccount] {
        &[]
    }

    fn books(&self) -> BookNames {
        BookNames::PLAIN
    }

    fn series(&self) -> &[Series] {
        &[]
    }

    fn migrations(&self) -> &[Migration] {
        &[]
    }

    fn mount(&self) -> Option<Mount> {
        None
    }

    fn templates(&self) -> &[Template] {
        &[]
    }
}

/// The built-in pack `none`: no tax codes and nothing added.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoPack;

impl Hooks for NoPack {}

impl Pack for NoPack {
    fn name(&self) -> &'static str {
        "none"
    }

    fn tax(&self) -> &dyn TaxPack {
        &NoTax
    }
}
