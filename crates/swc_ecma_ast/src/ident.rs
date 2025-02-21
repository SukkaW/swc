use std::{
    fmt::Display,
    ops::{Deref, DerefMut},
};

use phf::phf_set;
use scoped_tls::scoped_thread_local;
use swc_atoms::{js_word, Atom};
use swc_common::{
    ast_node, util::take::Take, BytePos, EqIgnoreSpan, Span, Spanned, SyntaxContext, DUMMY_SP,
};
use unicode_id::UnicodeID;

use crate::{typescript::TsTypeAnn, Expr};

/// Identifier used as a pattern.
#[derive(Spanned, Clone, Debug, PartialEq, Eq, Hash, EqIgnoreSpan)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[cfg_attr(
    any(feature = "rkyv-impl"),
    derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)
)]
#[cfg_attr(
    any(feature = "rkyv-impl"),
    archive(bound(
        serialize = "__S: rkyv::ser::Serializer + rkyv::ser::ScratchSpace + \
                     rkyv::ser::SharedSerializeRegistry",
        deserialize = "__D: rkyv::de::SharedDeserializeRegistry"
    ))
)]
#[cfg_attr(feature = "rkyv-impl", archive(check_bytes))]
#[cfg_attr(feature = "rkyv-impl", archive_attr(repr(C)))]
#[cfg_attr(feature = "serde-impl", derive(serde::Serialize, serde::Deserialize))]
pub struct BindingIdent {
    #[span]
    #[cfg_attr(feature = "serde-impl", serde(flatten))]
    #[cfg_attr(feature = "__rkyv", omit_bounds)]
    pub id: Ident,
    #[cfg_attr(feature = "serde-impl", serde(default, rename = "typeAnnotation"))]
    #[cfg_attr(feature = "__rkyv", omit_bounds)]
    pub type_ann: Option<Box<TsTypeAnn>>,
}

impl From<BindingIdent> for Box<Expr> {
    fn from(bi: BindingIdent) -> Self {
        Box::new(Expr::Ident(bi.id))
    }
}

impl Deref for BindingIdent {
    type Target = Ident;

    fn deref(&self) -> &Self::Target {
        &self.id
    }
}

impl DerefMut for BindingIdent {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.id
    }
}

impl BindingIdent {
    /// See [`Ident::to_id`] for documentation.
    pub fn to_id(&self) -> Id {
        self.id.to_id()
    }
}

impl Take for BindingIdent {
    fn dummy() -> Self {
        BindingIdent {
            id: Ident::dummy(),
            type_ann: None,
        }
    }
}

impl From<Ident> for BindingIdent {
    fn from(id: Ident) -> Self {
        Self { id, type_ann: None }
    }
}

bridge_from!(BindingIdent, Ident, Id);

/// A complete identifier with span.
///
/// Identifier of swc consists of two parts. The first one is symbol, which is
/// stored using an interned string, [Atom] . The second
/// one is [SyntaxContext][swc_common::SyntaxContext], which can be
/// used to distinguish identifier with same symbol.
///
/// Let me explain this with an example.
///
/// ```ts
/// let a = 5
/// {
///     let a = 3;
/// }
/// ```
/// In the code above, there are two variables with the symbol a.
///
///
/// Other compilers typically uses type like `Scope`, and store them nested, but
/// in rust, type like `Scope`  requires [Arc<Mutex<Scope>>] so swc uses
/// different approach. Instead of passing scopes, swc annotates two variables
/// with different tag, which is named
/// [SyntaxContext]. The notation for the syntax
/// context is #n where n is a number. e.g. `foo#1`
///
/// For the example above, after applying resolver pass, it becomes.
///
/// ```ts
/// let a#1 = 5
/// {
///     let a#2 = 3;
/// }
/// ```
///
/// Thanks to the `tag` we attached, we can now distinguish them.
///
/// ([Atom], [SyntaxContext])
///
/// See [Id], which is a type alias for this.
///
/// This can be used to store all variables in a module to single hash map.
///
/// # Comparison
///
/// While comparing two identifiers, you can use `.to_id()`.
///
/// # HashMap
///
/// There's a type named [Id] which only contains minimal information to
/// distinguish identifiers.
#[ast_node("Identifier")]
#[derive(Eq, Hash)]
pub struct Ident {
    pub span: Span,
    #[cfg_attr(feature = "serde-impl", serde(rename = "value"))]
    pub sym: Atom,

    /// TypeScript only. Used in case of an optional parameter.
    #[cfg_attr(feature = "serde-impl", serde(default))]
    pub optional: bool,
}

impl From<BindingIdent> for Ident {
    fn from(bi: BindingIdent) -> Self {
        bi.id
    }
}

impl From<&'_ str> for Ident {
    fn from(bi: &str) -> Self {
        Ident::new(bi.into(), DUMMY_SP)
    }
}

scoped_thread_local!(static EQ_IGNORE_SPAN_IGNORE_CTXT: ());

impl EqIgnoreSpan for Ident {
    fn eq_ignore_span(&self, other: &Self) -> bool {
        if self.sym != other.sym {
            return false;
        }

        if self.span.ctxt == other.span.ctxt {
            return true;
        }

        EQ_IGNORE_SPAN_IGNORE_CTXT.is_set()
    }
}

impl From<Id> for Ident {
    fn from(id: Id) -> Self {
        Ident::new(id.0, DUMMY_SP.with_ctxt(id.1))
    }
}

impl From<Ident> for Id {
    fn from(i: Ident) -> Self {
        (i.sym, i.span.ctxt)
    }
}

#[repr(C, align(64))]
struct Align64<T>(pub(crate) T);

const T: bool = true;
const F: bool = false;

impl Ident {
    /// In `op`, [EqIgnoreSpan] of [Ident] will ignore the syntax context.
    pub fn within_ignored_ctxt<F, Ret>(op: F) -> Ret
    where
        F: FnOnce() -> Ret,
    {
        EQ_IGNORE_SPAN_IGNORE_CTXT.set(&(), op)
    }

    /// Preserve syntax context while drop `span.lo` and `span.hi`.
    pub fn without_loc(mut self) -> Ident {
        self.span.lo = BytePos::DUMMY;
        self.span.hi = BytePos::DUMMY;
        self
    }

    /// Creates `Id` using `Atom` and `SyntaxContext` of `self`.
    pub fn to_id(&self) -> Id {
        (self.sym.clone(), self.span.ctxt)
    }

    /// Returns true if `c` is a valid character for an identifier start.
    #[inline]
    pub fn is_valid_start(c: char) -> bool {
        // This contains `$` (36) and `_` (95)
        const ASCII_START: Align64<[bool; 128]> = Align64([
            F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F,
            F, F, F, F, F, F, F, T, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F,
            F, F, F, F, F, F, F, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T,
            T, T, T, T, F, F, F, F, T, F, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T,
            T, T, T, T, T, T, T, F, F, F, F, F,
        ]);

        if c.is_ascii() {
            return ASCII_START.0[c as usize];
        }

        UnicodeID::is_id_start(c)
    }

    /// Returns true if `c` is a valid character for an identifier part after
    /// start.
    #[inline]
    pub fn is_valid_continue(c: char) -> bool {
        // This contains `$` (36)
        const ASCII_CONTINUE: Align64<[bool; 128]> = Align64([
            F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F,
            F, F, F, F, F, F, F, T, F, F, F, F, F, F, F, F, F, F, F, T, T, T, T, T, T, T, T, T, T,
            F, F, F, F, F, F, F, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T,
            T, T, T, T, F, F, F, F, T, F, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T, T,
            T, T, T, T, T, T, T, F, F, F, F, F,
        ]);

        if c.is_ascii() {
            return ASCII_CONTINUE.0[c as usize];
        }

        UnicodeID::is_id_continue(c) || c == '\u{200c}' || c == '\u{200d}'
    }

    /// Alternative for `toIdentifier` of babel.
    ///
    /// Returns [Ok] if it's a valid identifier and [Err] if it's not valid.
    /// The returned [Err] contains the valid symbol.
    pub fn verify_symbol(s: &str) -> Result<(), String> {
        fn is_reserved_symbol(s: &str) -> bool {
            s.is_reserved() || s.is_reserved_in_strict_mode(true) || s.is_reserved_in_strict_bind()
        }

        if is_reserved_symbol(s) {
            let mut buf = String::with_capacity(s.len() + 1);
            buf.push('_');
            buf.push_str(s);
            return Err(buf);
        }

        {
            let mut chars = s.chars();

            if let Some(first) = chars.next() {
                if Self::is_valid_start(first) && chars.all(Self::is_valid_continue) {
                    return Ok(());
                }
            }
        }

        let mut buf = String::with_capacity(s.len() + 2);
        let mut has_start = false;

        for c in s.chars() {
            if !has_start && Self::is_valid_start(c) {
                has_start = true;
                buf.push(c);
                continue;
            }

            if Self::is_valid_continue(c) {
                buf.push(c);
            }
        }

        if buf.is_empty() {
            buf.push('_');
        }

        if is_reserved_symbol(&buf) {
            let mut new_buf = String::with_capacity(buf.len() + 1);
            new_buf.push('_');
            new_buf.push_str(&buf);
            buf = new_buf;
        }

        Err(buf)
    }

    #[inline]
    pub fn is_dummy(&self) -> bool {
        self.sym == js_word!("") && self.span.is_dummy()
    }
}

/// See [Ident] for documentation.
pub type Id = (Atom, SyntaxContext);

impl Take for Ident {
    fn dummy() -> Self {
        Ident::new(js_word!(""), DUMMY_SP)
    }
}

impl Display for Ident {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{:?}", self.sym, self.span.ctxt)
    }
}

#[cfg(feature = "arbitrary")]
#[cfg_attr(docsrs, doc(cfg(feature = "arbitrary")))]
impl<'a> arbitrary::Arbitrary<'a> for Ident {
    fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
        let span = u.arbitrary()?;
        let sym = u.arbitrary::<String>()?;
        if sym.is_empty() {
            return Err(arbitrary::Error::NotEnoughData);
        }
        let sym = sym.into();

        let optional = u.arbitrary()?;

        Ok(Self {
            span,
            sym,
            optional,
        })
    }
}

#[ast_node("PrivateName")]
#[derive(Eq, Hash, EqIgnoreSpan)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct PrivateName {
    pub span: Span,
    pub id: Ident,
}

impl AsRef<str> for Ident {
    fn as_ref(&self) -> &str {
        &self.sym
    }
}

impl Ident {
    pub const fn new(sym: Atom, span: Span) -> Self {
        Ident {
            span,
            sym,
            optional: false,
        }
    }
}

static RESERVED: phf::Set<&str> = phf_set!(
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "null",
    "package",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
);

static RESSERVED_IN_STRICT_MODE: phf::Set<&str> = phf_set!(
    "implements",
    "interface",
    "let",
    "package",
    "private",
    "protected",
    "public",
    "static",
    "yield",
);

static RESERVED_IN_ES3: phf::Set<&str> = phf_set!(
    "abstract",
    "boolean",
    "byte",
    "char",
    "double",
    "final",
    "float",
    "goto",
    "int",
    "long",
    "native",
    "short",
    "synchronized",
    "throws",
    "transient",
    "volatile",
);

pub trait IdentExt: AsRef<str> {
    fn is_reserved(&self) -> bool {
        RESERVED.contains(self.as_ref())
    }

    fn is_reserved_in_strict_mode(&self, is_module: bool) -> bool {
        if is_module && self.as_ref() == "await" {
            return true;
        }
        RESSERVED_IN_STRICT_MODE.contains(self.as_ref())
    }

    fn is_reserved_in_strict_bind(&self) -> bool {
        ["eval", "arguments"].contains(&self.as_ref())
    }

    fn is_reserved_in_es3(&self) -> bool {
        RESERVED_IN_ES3.contains(self.as_ref())
    }
}

impl IdentExt for Atom {}
impl IdentExt for Ident {}
impl IdentExt for &'_ str {}
impl IdentExt for String {}
