//! Value types used in parameter structs.

/// A field of an update op that can be kept, set or cleared.
///
/// `#[op(clearable)]` on a `Patch<T>` field adds `--clear-<field>`. On JSON
/// paths `null` is [`Patch::Clear`] and an absent key is [`Patch::Keep`].
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Patch<T> {
    /// Leave the stored value as it is.
    #[default]
    Keep,
    /// Replace the stored value.
    Set(T),
    /// Remove the stored value.
    Clear,
}
