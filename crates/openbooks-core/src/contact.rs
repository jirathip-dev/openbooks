use crate::Refusal;

/// What a contact is to the company: a customer, a supplier, both, or an employee, who is neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContactKind {
    Customer,
    Supplier,
    CustomerAndSupplier,
    Employee,
}

impl ContactKind {
    pub const ALL: [Self; 4] = [
        Self::Customer,
        Self::Supplier,
        Self::CustomerAndSupplier,
        Self::Employee,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Customer => "customer",
            Self::Supplier => "supplier",
            Self::CustomerAndSupplier => "customer_and_supplier",
            Self::Employee => "employee",
        }
    }

    pub fn parse(text: &str) -> Result<Self, Refusal> {
        Self::ALL
            .into_iter()
            .find(|k| k.as_str() == text)
            .ok_or(Refusal::BadContactKind)
    }
}
