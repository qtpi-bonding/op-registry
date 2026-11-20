//! `Secret` and `SecretFile`: values that never print their contents.

use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

/// A value whose `Debug` and `Display` print `<secret>`.
#[derive(Clone)]
pub struct Secret<T>(T);

impl<T> Secret<T> {
    /// Wraps `value`.
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// The wrapped value. Callers keep it out of logs and errors.
    pub fn expose(&self) -> &T {
        &self.0
    }
}

impl<T> std::fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<secret>")
    }
}

impl<T> std::fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<secret>")
    }
}

/// Secret bytes read from a path, from stdin, or passed inline.
///
/// The bytes are zeroized on drop. `Debug` prints the path (or `<secret>`
/// when the field is `#[op(secret)]`) and never the bytes.
#[derive(Clone)]
pub struct SecretFile {
    path: Option<PathBuf>,
    show_path: bool,
    bytes: Secret<Zeroizing<Vec<u8>>>,
}

impl SecretFile {
    /// Secret bytes that came from `path` (`None` for stdin or inline bytes).
    pub fn new(path: Option<PathBuf>, bytes: Vec<u8>) -> Self {
        Self {
            path,
            show_path: true,
            bytes: Secret::new(Zeroizing::new(bytes)),
        }
    }

    /// The contents.
    pub fn bytes(&self) -> &[u8] {
        self.bytes.expose()
    }

    /// The path the bytes were read from, if any.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    #[doc(hidden)]
    pub fn hide_path(mut self) -> Self {
        self.show_path = false;
        self
    }

    pub(crate) fn path_is_secret(&self) -> bool {
        !self.show_path
    }
}

impl std::fmt::Debug for SecretFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let path: &dyn std::fmt::Debug = match (&self.path, self.show_path) {
            (Some(path), true) => path,
            (Some(_), false) => &"<secret>",
            (None, _) => &"<none>",
        };
        f.debug_struct("SecretFile")
            .field("path", path)
            .field("bytes", &self.bytes)
            .finish()
    }
}
