//! `assert_manual_snapshot`: the checked-in manual equals a fresh render.

use crate::manual::{render, Manual};
use crate::{CliContext, Registry};
use std::path::Path;

/// Compares a fresh render with the file at `path`. With `update` the file
/// is rewritten instead (and its directory created).
///
/// # Errors
///
/// A message naming the first differing line, or saying the file is missing,
/// that ends with how to update the snapshot.
pub fn check_manual_snapshot<C: CliContext>(
    registry: &Registry<C>,
    manual: &Manual,
    path: &Path,
    update: bool,
) -> Result<(), String> {
    let fresh =
        render(registry, manual).map_err(|err| format!("the manual does not render: {err:#}"))?;
    if update {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
        }
        return std::fs::write(path, fresh)
            .map_err(|err| format!("cannot write {}: {err}", path.display()));
    }
    let hint = "Run with UPDATE_SNAPSHOTS=1 to rewrite it.";
    let stored = std::fs::read_to_string(path)
        .map_err(|err| format!("cannot read the snapshot {}: {err}. {hint}", path.display()))?;
    if stored == fresh {
        return Ok(());
    }
    let (mut old, mut new) = (stored.lines(), fresh.lines());
    let mut line = 1;
    let (old_line, new_line) = loop {
        match (old.next(), new.next()) {
            (None, None) => {
                return Err(format!(
                    "the manual differs from {} only in line endings or the trailing newline\n{hint}",
                    path.display()
                ));
            }
            (a, b) if a == b => line += 1,
            (a, b) => break (a.unwrap_or("<end of file>"), b.unwrap_or("<end of file>")),
        }
    };
    Err(format!(
        "the manual differs from {} at line {line}\n  snapshot: {old_line}\n  fresh:    {new_line}\n{hint}",
        path.display()
    ))
}

/// Panics when the manual differs from the snapshot at `path`; with
/// `UPDATE_SNAPSHOTS=1` in the environment it rewrites the file instead.
pub fn assert_manual_snapshot<C: CliContext>(registry: &Registry<C>, manual: &Manual, path: &Path) {
    let update = std::env::var("UPDATE_SNAPSHOTS").is_ok_and(|v| !v.is_empty() && v != "0");
    if let Err(message) = check_manual_snapshot(registry, manual, path, update) {
        panic!("assert_manual_snapshot: {message}");
    }
}
