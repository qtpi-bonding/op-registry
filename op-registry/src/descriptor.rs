//! Data types used to describe registered operations.

use coded_error::ErrorClass;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The capabilities an operation may use.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Access {
    /// Whether the operation reads data.
    pub reads: bool,
    /// Whether the operation writes data.
    pub writes: bool,
    /// Whether the operation accesses the network.
    pub network: bool,
    /// Whether the operation spawns work.
    pub spawns: bool,
    /// Whether the operation is destructive.
    pub destructive: bool,
}

impl Access {
    /// Creates access metadata with read access enabled.
    pub const fn reads() -> Self {
        Self {
            reads: true,
            writes: false,
            network: false,
            spawns: false,
            destructive: false,
        }
    }

    /// Creates access metadata with write access enabled.
    pub const fn writes() -> Self {
        Self {
            reads: false,
            writes: true,
            network: false,
            spawns: false,
            destructive: false,
        }
    }

    /// Creates access metadata for a destructive operation (it also writes).
    pub const fn destructive() -> Self {
        Self {
            reads: false,
            writes: true,
            network: false,
            spawns: false,
            destructive: true,
        }
    }

    /// Enables network access.
    pub fn with_network(mut self) -> Self {
        self.network = true;
        self
    }

    /// Enables spawning work.
    pub fn with_spawns(mut self) -> Self {
        self.spawns = true;
        self
    }

    /// Marks the access as destructive.
    pub fn with_destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    /// Builds access from words: `reads`, `writes`, `network`, `spawns` and
    /// `destructive` (which also sets `writes`).
    ///
    /// # Panics
    ///
    /// Panics on any other word.
    pub fn from_words(words: &[&str]) -> Self {
        let mut access = Self::default();
        for word in words {
            match *word {
                "reads" => access.reads = true,
                "writes" => access.writes = true,
                "network" => access.network = true,
                "spawns" => access.spawns = true,
                "destructive" => {
                    access.destructive = true;
                    access.writes = true;
                }
                other => panic!("unknown access word {other:?}"),
            }
        }
        access
    }
}

/// The audience for an operation.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Audience {
    /// An operation intended for an agent.
    #[default]
    Agent,
    /// An operation intended for an owner.
    Owner,
    /// An operation intended for development use.
    Dev,
}

/// The shape of an operation.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    /// One request, one result.
    #[default]
    Unary,
    /// Like `Unary`, but runs in the calling process.
    Local,
    /// A stream of results.
    Stream,
    /// A list that returns every item.
    ListAll,
    /// A list that returns one page.
    ListPage,
}

/// One way of reaching an operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// The command line.
    Cli,
    /// A typed Rust call.
    Typed,
    /// An MCP tool call.
    Mcp,
    /// A JSON call: `Registry::call` or a JSON transport.
    Json,
}

impl Surface {
    const ORDER: [Surface; 4] = [Surface::Cli, Surface::Typed, Surface::Mcp, Surface::Json];

    const fn bit(self) -> u8 {
        1 << self as u8
    }

    /// The word the manual and the `paths` list use.
    pub fn word(self) -> &'static str {
        match self {
            Surface::Cli => "cli",
            Surface::Typed => "typed",
            Surface::Mcp => "mcp",
            Surface::Json => "json",
        }
    }
}

/// The set of surfaces that reach an operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Paths(u8);

impl Paths {
    /// Every surface.
    pub const ALL: Paths = Paths(0b1111);

    /// The set holding exactly `surfaces`.
    pub fn of(surfaces: &[Surface]) -> Paths {
        Paths(surfaces.iter().fold(0, |bits, s| bits | s.bit()))
    }

    /// Whether `s` is in the set.
    pub fn contains(self, s: Surface) -> bool {
        self.0 & s.bit() != 0
    }

    /// The surfaces in the set, in the order cli, typed, mcp, json.
    pub fn iter(self) -> impl Iterator<Item = Surface> {
        Surface::ORDER
            .into_iter()
            .filter(move |s| self.contains(*s))
    }
}

impl Default for Paths {
    fn default() -> Self {
        Paths::ALL
    }
}

impl Serialize for Paths {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter())
    }
}

impl<'de> Deserialize<'de> for Paths {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let surfaces = Vec::<Surface>::deserialize(deserializer)?;
        Ok(Paths::of(&surfaces))
    }
}

/// A documented exit code.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Exit {
    /// The process exit code.
    pub code: u8,
    /// What the code means.
    pub meaning: String,
}

/// A serialisable copy of [`coded_error::CodeEntry`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ErrorCodeEntry {
    /// The stable, namespaced error code.
    pub code: String,
    /// The class, or `None` when it is decided at runtime.
    pub class: Option<ErrorClass>,
}

impl From<&coded_error::CodeEntry> for ErrorCodeEntry {
    fn from(entry: &coded_error::CodeEntry) -> Self {
        Self {
            code: entry.code.to_owned(),
            class: entry.class,
        }
    }
}

/// The serializable metadata describing an operation.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Descriptor {
    /// The operation name, derived from the path by [`derive_name`].
    pub name: String,
    /// The command words, outermost group first and the verb last.
    pub path: Vec<String>,
    /// A one-line summary.
    pub summary: String,
    /// Longer help text.
    pub long_help: String,
    /// The operation's access requirements.
    pub access: Access,
    /// The intended audience.
    pub audience: Audience,
    /// Every audience the operation belongs to, the primary first, no duplicates.
    pub audiences: Vec<Audience>,
    /// The shape of the operation.
    pub kind: OpKind,
    /// Whether the operation is hidden from listings.
    pub hidden: bool,
    /// Whether the command runs when its group is named without a verb.
    pub group_default: bool,
    /// Whether the operation has a text rendering.
    pub text: bool,
    /// Documented exit codes.
    pub exits: Vec<Exit>,
    /// Error codes the operation may return.
    pub errors: Vec<ErrorCodeEntry>,
    /// Which call paths reach the operation.
    pub paths: Paths,
    /// Whether the operation accepts a cursor.
    pub cursor: bool,
    /// The input schema.
    pub input_schema: Value,
    /// The output schema, when the `schema` feature is on.
    pub output_schema: Option<Value>,
    /// For a list pair, the name of the other operation.
    pub list_pair: Option<String>,
    /// Whether the op declares an unpaged form (list pairs, Task 9).
    pub unpaged: bool,
    /// The long flags (without dashes) of the globals the op needs on the command line.
    pub needs_globals: Vec<String>,
}

impl Descriptor {
    /// The last word of the path.
    pub fn verb(&self) -> &str {
        self.path.last().map(String::as_str).unwrap_or_default()
    }

    /// Every word of the path but the last.
    pub fn noun_path(&self) -> &[String] {
        match self.path.split_last() {
            Some((_, noun)) => noun,
            None => &[],
        }
    }
}

/// Derives an operation name from its path: the words joined with `_`, with
/// every `-` turned into `_`.
pub fn derive_name(path: &[String]) -> String {
    path.join("_").replace('-', "_")
}
