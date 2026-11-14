//! Shared status types.
//!
//! These types form the wire contract for `status get`. Field order is part
//! of the contract: JSON output follows declaration order.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Health state of a component.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "registry", derive(schemars::JsonSchema))]
#[non_exhaustive]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum State {
    /// The state has not been reported.
    Unspecified,
    /// The state is known to exist but cannot be determined.
    Unknown,
    /// The component is starting up.
    Starting,
    /// The component is healthy.
    Ok,
    /// The component works with reduced capability.
    Degraded,
    /// The component is paused on purpose.
    Paused,
    /// The component is down.
    Down,
    /// The component has stopped.
    Stopped,
}

/// A scalar attribute value. Numbers stay numbers in JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "registry", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum AttrValue {
    /// A string value.
    Str(String),
    /// A signed integer value.
    Int(i64),
    /// A floating point value.
    Float(f64),
    /// A boolean value.
    Bool(bool),
}

impl From<&str> for AttrValue {
    fn from(value: &str) -> Self {
        AttrValue::Str(value.to_owned())
    }
}

impl From<String> for AttrValue {
    fn from(value: String) -> Self {
        AttrValue::Str(value)
    }
}

impl From<i64> for AttrValue {
    fn from(value: i64) -> Self {
        AttrValue::Int(value)
    }
}

impl From<f64> for AttrValue {
    fn from(value: f64) -> Self {
        AttrValue::Float(value)
    }
}

impl From<bool> for AttrValue {
    fn from(value: bool) -> Self {
        AttrValue::Bool(value)
    }
}

/// Reference to a parent component by kind and id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "registry", derive(schemars::JsonSchema))]
pub struct ComponentRef {
    /// Kind of the referenced component.
    pub kind: String,
    /// Id of the referenced component.
    pub id: String,
}

/// A single reported component.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "registry", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct Component {
    /// Kind of the component.
    pub kind: String,
    /// Id of the component, unique within its kind.
    pub id: String,
    /// Optional parent component.
    pub parent: Option<ComponentRef>,
    /// Current health state.
    pub state: State,
    /// Whether the component is ready to serve.
    pub ready: bool,
    /// Whether the component has reached a terminal state.
    pub terminal: bool,
    /// Code of the last error, empty when none.
    pub last_error_code: String,
    /// Message of the last error, empty when none.
    pub last_error_message: String,
    /// Consecutive failure count, when tracked.
    pub consecutive_failures: Option<u32>,
    /// Time of the last state change, in Unix milliseconds.
    pub changed_at_unix_ms: i64,
    /// Allow-listed scalar attributes.
    pub attrs: BTreeMap<String, AttrValue>,
}

impl Component {
    /// Creates a component with empty optional data.
    pub fn new(kind: impl Into<String>, id: impl Into<String>, state: State) -> Self {
        Component {
            kind: kind.into(),
            id: id.into(),
            parent: None,
            state,
            ready: false,
            terminal: false,
            last_error_code: String::new(),
            last_error_message: String::new(),
            consecutive_failures: None,
            changed_at_unix_ms: 0,
            attrs: BTreeMap::new(),
        }
    }

    /// Sets the parent component.
    pub fn parent(mut self, kind: impl Into<String>, id: impl Into<String>) -> Self {
        self.parent = Some(ComponentRef {
            kind: kind.into(),
            id: id.into(),
        });
        self
    }

    /// Sets the ready flag.
    pub fn ready(mut self, ready: bool) -> Self {
        self.ready = ready;
        self
    }

    /// Sets the terminal flag.
    pub fn terminal(mut self, terminal: bool) -> Self {
        self.terminal = terminal;
        self
    }

    /// Sets the last error code and message.
    pub fn last_error(mut self, code: impl Into<String>, message: impl Into<String>) -> Self {
        self.last_error_code = code.into();
        self.last_error_message = message.into();
        self
    }

    /// Sets the consecutive failure count.
    pub fn consecutive_failures(mut self, count: u32) -> Self {
        self.consecutive_failures = Some(count);
        self
    }

    /// Sets the time of the last state change.
    pub fn changed_at_unix_ms(mut self, ms: i64) -> Self {
        self.changed_at_unix_ms = ms;
        self
    }

    /// Adds one attribute.
    pub fn attr(mut self, key: impl Into<String>, value: impl Into<AttrValue>) -> Self {
        self.attrs.insert(key.into(), value.into());
        self
    }
}

/// A point-in-time view of a service's components.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "registry", derive(schemars::JsonSchema))]
pub struct StatusSnapshot {
    /// Service name.
    pub service: String,
    /// Service version.
    pub version: String,
    /// Time the snapshot was taken, in Unix milliseconds.
    pub taken_at_unix_ms: i64,
    /// Source epoch at snapshot time.
    pub epoch: u64,
    /// Returned components.
    pub components: Vec<Component>,
    /// True when more components matched than were returned.
    pub truncated: bool,
}

/// Filter applied when taking a snapshot.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "registry", derive(schemars::JsonSchema))]
pub struct StatusFilter {
    /// Only components of this kind.
    pub kind: Option<String>,
    /// Only components with this id.
    pub id: Option<String>,
}

/// Identity of a service.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "registry", derive(schemars::JsonSchema))]
pub struct ServiceInfo {
    /// Service name.
    pub name: String,
    /// Service version.
    pub version: String,
}
