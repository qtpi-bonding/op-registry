//! Data types used to describe registered operations.

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

/// The shape of an operation call.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum CallShape {
    /// An operation that returns one result.
    #[default]
    Unary,
    /// An operation that returns a stream of results.
    Stream,
}

/// The serializable metadata describing an operation.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Descriptor {
    /// The operation name, explicit or derived from noun and verb.
    pub name: String,
    /// The operation's noun.
    pub noun: String,
    /// The operation's verb.
    pub verb: String,
    /// A description of the operation.
    pub description: String,
    /// The operation's access requirements.
    pub access: Access,
    /// The intended audience.
    pub audience: Audience,
    /// Whether the operation produces a stream.
    pub streaming: bool,
    /// The input schema.
    pub input_schema: Value,
    /// The output schema.
    pub output_schema: Value,
    /// Whether the result renders as text.
    pub renders_text: bool,
    /// Whether a successful operation supplies an exit code.
    pub exit_code_on_ok: bool,
    /// Required capabilities.
    /// Declared, not acted on yet.
    pub requires: Vec<String>,
    /// Whether the operation is hidden.
    /// Declared, not acted on yet.
    pub hidden: bool,
    /// The version in which the operation was introduced.
    /// Declared, not acted on yet.
    pub since: Option<String>,
    /// The deprecation notice for the operation.
    /// Declared, not acted on yet.
    pub deprecated: Option<String>,
    /// Alternative names for the operation.
    /// Declared, not acted on yet.
    pub aliases: Vec<String>,
}

impl Descriptor {
    /// Creates a descriptor and derives its name from its noun and verb.
    pub fn new(noun: impl Into<String>, verb: impl Into<String>) -> Self {
        let noun = noun.into();
        let verb = verb.into();
        let name = derive_name(&noun, &verb);
        Self {
            name,
            noun,
            verb,
            ..Self::default()
        }
    }

    /// Overrides the descriptor's derived name.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
    /// Sets the operation description.
    pub fn description(mut self, value: impl Into<String>) -> Self {
        self.description = value.into();
        self
    }
    /// Sets the operation access metadata.
    pub fn access(mut self, value: Access) -> Self {
        self.access = value;
        self
    }
    /// Sets the operation audience.
    pub fn audience(mut self, value: Audience) -> Self {
        self.audience = value;
        self
    }
    /// Sets whether the operation streams results.
    pub fn streaming(mut self, value: bool) -> Self {
        self.streaming = value;
        self
    }
    /// Sets the input schema.
    pub fn input_schema(mut self, value: Value) -> Self {
        self.input_schema = value;
        self
    }
    /// Sets the output schema.
    pub fn output_schema(mut self, value: Value) -> Self {
        self.output_schema = value;
        self
    }
    /// Sets whether the result renders as text.
    pub fn renders_text(mut self, value: bool) -> Self {
        self.renders_text = value;
        self
    }
    /// Sets whether success supplies an exit code.
    pub fn exit_code_on_ok(mut self, value: bool) -> Self {
        self.exit_code_on_ok = value;
        self
    }
    /// Sets the required capabilities.
    pub fn requires(mut self, value: Vec<String>) -> Self {
        self.requires = value;
        self
    }
    /// Sets whether the operation is hidden.
    pub fn hidden(mut self, value: bool) -> Self {
        self.hidden = value;
        self
    }
    /// Sets the introduction version.
    pub fn since(mut self, value: impl Into<String>) -> Self {
        self.since = Some(value.into());
        self
    }
    /// Sets the deprecation notice.
    pub fn deprecated(mut self, value: impl Into<String>) -> Self {
        self.deprecated = Some(value.into());
        self
    }
    /// Sets alternative operation names.
    pub fn aliases(mut self, value: Vec<String>) -> Self {
        self.aliases = value;
        self
    }

    /// Returns the name used by a command-line interface.
    pub fn cli_name(&self) -> String {
        if self.name != derive_name(&self.noun, &self.verb) {
            self.name.clone()
        } else {
            let words: Vec<&str> = self.noun.split_whitespace().collect();
            format!("{} {}", words.join(" "), self.verb)
        }
    }
}

/// Derives an operation name from a noun path and a verb.
///
/// The noun's whitespace-separated words are joined with `_`, every `-` becomes
/// `_`, and the verb is appended after another `_` with its own `-` also turned
/// into `_` (the CLI keeps the verb as written).
fn derive_name(noun: &str, verb: &str) -> String {
    let words: Vec<&str> = noun.split_whitespace().collect();
    // A single-word noun is kept exactly as written; only paths are normalised.
    let path = if words.len() == 1 {
        words[0].to_owned()
    } else {
        words.join("_").replace('-', "_")
    };
    format!("{path}_{}", verb.replace('-', "_"))
}
