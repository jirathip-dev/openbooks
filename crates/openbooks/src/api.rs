use openbooks_core::{Email, OrgId, Role, UserId, Word};
use openbooks_db::members::Member;

/// How an operation reaches its callers: a read, a write offered as a tool, or a write closed to tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Read,
    Write,
    Closed,
}

/// One route, made only by `read!`, `write!` or `closed!`, with the word it needs and the shapes it takes and answers.
#[doc = "```compile_fail,E0451"]
#[doc = "use openbooks::{Kind, Operation, Schema};"]
#[doc = "let undeclared = Operation { kind: Kind::Read, name: \"x\", word: openbooks_core::Word::Read, about: \"x\", input: Vec::new, output: || Schema::Nothing };"]
#[doc = "```"]
#[derive(Debug, Clone, Copy)]
pub struct Operation {
    kind: Kind,
    name: &'static str,
    word: Word,
    about: &'static str,
    input: fn() -> Vec<Field>,
    output: fn() -> Schema,
}

impl Operation {
    #[doc(hidden)]
    pub const fn declared(
        kind: Kind,
        name: &'static str,
        word: Word,
        about: &'static str,
        input: fn() -> Vec<Field>,
        output: fn() -> Schema,
    ) -> Self {
        Self {
            kind,
            name,
            word,
            about,
            input,
            output,
        }
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn word(&self) -> Word {
        self.word
    }

    pub fn about(&self) -> &'static str {
        self.about
    }

    pub fn input(&self) -> Vec<Field> {
        (self.input)()
    }

    pub fn output(&self) -> Schema {
        (self.output)()
    }

    pub fn is_tool(&self) -> bool {
        self.kind != Kind::Closed
    }
}

/// The operations a host mounts under one key; `openapi` writes their document.
#[derive(Debug, Clone, Copy)]
pub struct Mount {
    pub key: &'static str,
    pub operations: &'static [Operation],
}

/// The shape of a value on the wire: each field class with its schema marks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Schema {
    Nothing,
    Text,
    Email,
    Id,
    Words(Vec<&'static str>),
    Object(Vec<Field>),
    Maybe(Box<Schema>),
    Page(Box<Schema>),
    Never(Box<Schema>),
}

/// A named field of an object with what it means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: &'static str,
    pub about: &'static str,
    pub schema: Schema,
}

/// A type that says its own shape on the wire.
pub trait Shape {
    fn schema() -> Schema;
}

/// One page of a list, with the cursor to pass as `after` for the next page while there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next: Option<String>,
}

/// A value of the never tier: a person may see it, a tool's answer never carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Never<T>(pub T);

impl Shape for () {
    fn schema() -> Schema {
        Schema::Nothing
    }
}

impl Shape for String {
    fn schema() -> Schema {
        Schema::Text
    }
}

impl Shape for Email {
    fn schema() -> Schema {
        Schema::Email
    }
}

impl Shape for OrgId {
    fn schema() -> Schema {
        Schema::Id
    }
}

impl Shape for UserId {
    fn schema() -> Schema {
        Schema::Id
    }
}

impl Shape for Role {
    fn schema() -> Schema {
        Schema::Words(Role::ALL.map(Role::as_str).to_vec())
    }
}

impl Shape for Word {
    fn schema() -> Schema {
        Schema::Words(Word::ALL.map(Word::as_str).to_vec())
    }
}

impl<T: Shape> Shape for Option<T> {
    fn schema() -> Schema {
        Schema::Maybe(Box::new(T::schema()))
    }
}

impl<T: Shape> Shape for Page<T> {
    fn schema() -> Schema {
        Schema::Page(Box::new(T::schema()))
    }
}

impl<T: Shape> Shape for Never<T> {
    fn schema() -> Schema {
        Schema::Never(Box::new(T::schema()))
    }
}

impl Shape for Member {
    fn schema() -> Schema {
        Schema::Object(vec![
            Field {
                name: "email",
                about: "The address the member was added by.",
                schema: Email::schema(),
            },
            Field {
                name: "user",
                about: "The user id they signed in with, once they have.",
                schema: Option::<UserId>::schema(),
            },
            Field {
                name: "role",
                about: "Their role in the company.",
                schema: Role::schema(),
            },
        ])
    }
}

#[doc(hidden)]
#[macro_export]
macro_rules! operation {
    ($kind:ident, $name:literal, $word:expr, $about:literal, { $($field:ident: $ty:ty = $doc:literal),* $(,)? } -> $out:ty) => {
        $crate::Operation::declared(
            $crate::Kind::$kind,
            $name,
            $word,
            $about,
            || vec![$($crate::Field { name: stringify!($field), about: $doc, schema: <$ty as $crate::Shape>::schema() }),*],
            <$out as $crate::Shape>::schema,
        )
    };
}

/// Declares a read: `read!("name", word, "what it does", { field: Type = "what it is" } -> Answer)`.
#[macro_export]
macro_rules! read {
    ($($t:tt)*) => { $crate::operation!(Read, $($t)*) };
}

/// Declares a write a tool may make; it leaves one audit row.
#[macro_export]
macro_rules! write {
    ($($t:tt)*) => { $crate::operation!(Write, $($t)*) };
}

/// Declares a write closed to tools: only a person makes it; it leaves one audit row.
#[macro_export]
macro_rules! closed {
    ($($t:tt)*) => { $crate::operation!(Closed, $($t)*) };
}
