//! Attribute checks for status components.
//!
//! Privacy is by allow-list only. A component may carry attributes, but only
//! keys that a [`crate::StatusSource`] lists in `allowed_attr_keys` are passed on.
//!
//! Status output must never contain:
//!
//! - message bodies,
//! - prompts,
//! - room ids,
//! - transcript or node output,
//! - keys or tokens,
//! - policy docs,
//! - file paths or session metadata.
//!
//! This module is read-only. It inspects components and never changes them.

use crate::types::{AttrValue, Component};

/// Maximum number of attributes on one component.
pub const MAX_ATTRS: usize = 32;

/// Maximum length of an attribute key, in bytes.
pub const MAX_ATTR_KEY_BYTES: usize = 64;

/// Maximum length of a string attribute value, in bytes.
pub const MAX_ATTR_VALUE_BYTES: usize = 256;

/// Errors raised when a component's attributes break the status rules.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AttrError {
    #[error("attribute key {key} is not on the allow-list")]
    NotAllowed { key: String },
    #[error("component has {count} attributes, the maximum is {max}")]
    TooMany { count: usize, max: usize },
    #[error("attribute key {key} is {len} bytes, the maximum is {max}")]
    KeyTooLong { key: String, len: usize, max: usize },
    #[error("attribute {key} value is {len} bytes, the maximum is {max}")]
    ValueTooLong { key: String, len: usize, max: usize },
    #[error("field {field} is {len} bytes, the maximum is {max}")]
    FieldTooLong {
        field: String,
        len: usize,
        max: usize,
    },
}

/// Checks that a free-text field is at most `MAX_ATTR_VALUE_BYTES` bytes long.
fn check_field_len(field: &str, text: &str) -> Result<(), AttrError> {
    let len = text.len();
    if len > MAX_ATTR_VALUE_BYTES {
        return Err(AttrError::FieldTooLong {
            field: field.to_owned(),
            len,
            max: MAX_ATTR_VALUE_BYTES,
        });
    }
    Ok(())
}

/// Checks the service name and version against the free-text length bound.
pub fn check_service_info(info: &crate::types::ServiceInfo) -> Result<(), AttrError> {
    check_field_len("service", &info.name)?;
    check_field_len("version", &info.version)
}

/// Truncates `text` to the largest prefix of at most `max` bytes that ends on a char boundary.
pub fn truncate_at_char_boundary(text: &mut String, max: usize) {
    if text.len() <= max {
        return;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
}

impl coded_error::CodedError for AttrError {
    fn code(&self) -> &'static str {
        match self {
            Self::NotAllowed { .. } => "op_status.attr_not_allowed",
            Self::TooMany { .. } => "op_status.attr_too_many",
            Self::KeyTooLong { .. } => "op_status.attr_key_too_long",
            Self::ValueTooLong { .. } => "op_status.attr_value_too_long",
            Self::FieldTooLong { .. } => "op_status.field_too_long",
        }
    }

    fn class(&self) -> coded_error::ErrorClass {
        coded_error::ErrorClass::Logical
    }
}

/// Checks one component's attributes against the count, length and allow-list rules.
///
/// Keys are checked in `BTreeMap` order. The first failing rule is returned.
pub fn check_component(component: &Component, allowed: &[&str]) -> Result<(), AttrError> {
    check_field_len("kind", &component.kind)?;
    check_field_len("id", &component.id)?;
    check_field_len("last_error_code", &component.last_error_code)?;
    check_field_len("last_error_message", &component.last_error_message)?;
    let count = component.attrs.len();
    if count > MAX_ATTRS {
        return Err(AttrError::TooMany {
            count,
            max: MAX_ATTRS,
        });
    }
    for (key, value) in &component.attrs {
        if key.len() > MAX_ATTR_KEY_BYTES {
            return Err(AttrError::KeyTooLong {
                key: key.clone(),
                len: key.len(),
                max: MAX_ATTR_KEY_BYTES,
            });
        }
        if !allowed.contains(&key.as_str()) {
            return Err(AttrError::NotAllowed { key: key.clone() });
        }
        if let AttrValue::Str(text) = value {
            if text.len() > MAX_ATTR_VALUE_BYTES {
                return Err(AttrError::ValueTooLong {
                    key: key.clone(),
                    len: text.len(),
                    max: MAX_ATTR_VALUE_BYTES,
                });
            }
        }
    }
    Ok(())
}

/// Panics when any observed attribute key is not in `source_keys`.
///
/// Intended for source tests. The panic message names the offending key.
pub fn assert_allowed_keys<'a>(source_keys: &[&str], observed: impl IntoIterator<Item = &'a str>) {
    for key in observed {
        assert!(
            source_keys.contains(&key),
            "attribute key {key} is not in the source allow-list"
        );
    }
}
