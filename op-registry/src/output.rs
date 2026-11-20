//! Output schema discovery: `Some(schema)` when the `schema` feature is on.

/// Produces the JSON schema of an operation output type, when available.
pub trait OutputSchema {
    fn output_schema() -> Option<serde_json::Value>;
}

#[cfg(feature = "schema")]
impl<T: schemars::JsonSchema> OutputSchema for T {
    fn output_schema() -> Option<serde_json::Value> {
        Some(serde_json::to_value(schemars::schema_for!(T)).expect("schemars output is valid JSON"))
    }
}

#[cfg(not(feature = "schema"))]
impl<T> OutputSchema for T {
    fn output_schema() -> Option<serde_json::Value> {
        None
    }
}

/// Writes `bytes` to `path` for key material: a new file at mode 0600 on
/// unix. An existing file is an `AlreadyExists` error unless `overwrite`, in
/// which case it is truncated and its mode set to 0600.
pub fn write_private(
    path: &std::path::Path,
    bytes: &[u8],
    overwrite: bool,
) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    if overwrite {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)
}
