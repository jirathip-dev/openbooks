use crate::Refusal;

/// The one permission an operation needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Word {
    Read,
    Reports,
    Draft,
    Approve,
    Own,
}

impl Word {
    pub const ALL: [Self; 5] = [Self::Read, Self::Reports, Self::Draft, Self::Approve, Self::Own];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Reports => "reports",
            Self::Draft => "draft",
            Self::Approve => "approve",
            Self::Own => "own",
        }
    }
}

/// A member's fixed role in a company: a fixed set of words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    Owner,
    Accountant,
    Staff,
    Viewer,
}

impl Role {
    pub const ALL: [Self; 4] = [Self::Owner, Self::Accountant, Self::Staff, Self::Viewer];

    pub fn words(self) -> &'static [Word] {
        match self {
            Self::Owner => &Word::ALL,
            Self::Accountant => &[Word::Read, Word::Reports, Word::Draft, Word::Approve],
            Self::Staff => &[Word::Read, Word::Draft],
            Self::Viewer => &[Word::Read, Word::Reports],
        }
    }

    pub fn allows(self, word: Word) -> bool {
        self.words().contains(&word)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Accountant => "accountant",
            Self::Staff => "staff",
            Self::Viewer => "viewer",
        }
    }

    pub fn parse(text: &str) -> Result<Self, Refusal> {
        Self::ALL
            .into_iter()
            .find(|r| r.as_str() == text)
            .ok_or(Refusal::BadRole)
    }
}

/// An email address, trimmed and in lower case, the handle a member is added by.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Email(String);

impl Email {
    pub fn new(text: &str) -> Result<Self, Refusal> {
        let text = text.trim().to_lowercase();
        match text.split_once('@') {
            Some((name, domain)) if !name.is_empty() && domain.contains('.') && !domain.contains('@') => Ok(Self(text)),
            _ => Err(Refusal::BadEmail),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
