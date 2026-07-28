/// The Rust-side reader for the `threadbox.ir.v1` graph `guest/assembly/emit.ts`
/// writes. See `IR.md` for the specification this module implements: the
/// envelope, the twelve node kinds, parent-order rules, and the four
/// validators. This reader is deliberately hand-rolled and tactical — no
/// serialization crate, no bidirectional round-trip. It is regenerated
/// whenever the schema changes; see `IR.md` — Changing the schema.
mod error;
mod json;
mod parse;
mod validate;

pub use error::ParseError;

/// One of the value forms `Type` can carry. Named here rather than as a
/// raw string so a reader mistake ("litteral") is a compile error, not a
/// silent no-match at validation time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Literal,
    Field,
}

/// The optional model-selection record on `Locate` and `Verify`. Seven
/// slots, all optional; absent slots impose no constraint. See `IR.md` —
/// `ModelSpec` — for the emitted slot order (irrelevant to reading, since
/// this reader accepts fields in any order) and the two resolution
/// shapes (policy path vs. catalog filter path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSpec {
    pub role: Option<String>,
    pub tier: Option<String>,
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub think: Option<String>,
    pub context_window: Option<i64>,
    pub driver: Option<String>,
}

/// The twelve node kinds `IR.md` documents, each carrying exactly the
/// extra fields its row in that table names. `parents` is common to
/// every node and lives on `Node`, not here — see `Node`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    LoadJson { name: String },
    Screenshot,
    Scale { width: i64 },
    Locate { description: String, model: Option<ModelSpec> },
    Click,
    Type { value_kind: ValueKind, value: String },
    Verify { assertion: String, model: Option<ModelSpec> },
    Retry { bound: i64 },
    Fallback,
    Branch,
    ForEach { over: String, body: usize },
    Publish,
}

impl NodeKind {
    /// The kind's name exactly as `IR.md` spells it, for error messages.
    pub fn name(&self) -> &'static str {
        match self {
            NodeKind::LoadJson { .. } => "LoadJson",
            NodeKind::Screenshot => "Screenshot",
            NodeKind::Scale { .. } => "Scale",
            NodeKind::Locate { .. } => "Locate",
            NodeKind::Click => "Click",
            NodeKind::Type { .. } => "Type",
            NodeKind::Verify { .. } => "Verify",
            NodeKind::Retry { .. } => "Retry",
            NodeKind::Fallback => "Fallback",
            NodeKind::Branch => "Branch",
            NodeKind::ForEach { .. } => "ForEach",
            NodeKind::Publish => "Publish",
        }
    }
}

/// One arena slot: its own index, its kind, and the parent indices it
/// consumes. `ForEach.body` is deliberately not a parent — see `IR.md` —
/// `ForEach.body` — it is a field of `NodeKind::ForEach` instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub index: usize,
    pub kind: NodeKind,
    pub parents: Vec<usize>,
}

/// The whole arena, in ascending index order. `nodes[i].index == i` is
/// guaranteed once `parse()` returns `Ok`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Graph {
    pub nodes: Vec<Node>,
}

/// Parse a `threadbox.ir.v1` envelope, enforcing the structural
/// invariants `IR.md` lists before any validator runs: the `ir` tag, the
/// `nodes[i].i == i` identity, in-bounds and backward-pointing parent and
/// `body` indices, and a known `kind`. Does not run the four validators —
/// call `validate()` on the result for that.
pub fn parse(source: &str) -> Result<Graph, ParseError> {
    parse::parse_graph(source)
}

/// Run the four structural validators `IR.md` specifies, in that exact
/// order, stopping at the first failure.
pub fn validate(graph: &Graph) -> Result<(), ParseError> {
    validate::validate_graph(graph)
}
