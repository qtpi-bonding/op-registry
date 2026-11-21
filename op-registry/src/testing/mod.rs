//! Checks the library gives every consumer, behind the `testing` feature.
//!
//! Each `assert_*` panics with a report of every problem it found, never only
//! the first.

mod casing;
mod codes;
mod help;
mod offline;
mod parity;
mod refs;
mod sample;
mod secrets;
mod snapshot;

pub use casing::{assert_casing, Casing};
pub(crate) use codes::check_strict;
pub use codes::{assert_error_codes, assert_exit_table, check_exit_table, strict_codes, Fixture};
pub use help::{assert_help_complete, HelpRules};
pub use offline::assert_local_ops_offline;
pub use parity::{assert_canonical, assert_parity, parity_report, Mount, ParityLine, Reach};
pub use refs::assert_unknown_ref_not_found;
pub use sample::{sample_params, sample_params_seeded};
pub use secrets::{assert_secrets_not_echoed, SecretSentinel};
pub use snapshot::{assert_manual_snapshot, check_manual_snapshot};

/// The longest a kit call of a stream op may run without an item or an end.
pub(crate) const KIT_STREAM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The options every kit call carries: reconnecting streams give up after
/// their first failed connection, and a caller, so ops that need one (listen
/// ops) reach their body.
pub(crate) fn kit_options() -> crate::CallOptions {
    crate::CallOptions::default()
        .with_max_retries(0)
        .with_caller(crate::Caller(KIT_CALLER.to_owned()))
}

/// The caller label of every kit call, and the JSON transport name.
const KIT_CALLER: &str = "op-registry-kit";

/// The path the kit calls `d` on: the first of json, mcp and cli that `d`
/// declares. `None` when `d` is declared on typed only, which the kit cannot
/// call.
pub(crate) fn kit_path(d: &crate::Descriptor) -> Option<crate::CallPath> {
    use crate::{CallPath, Surface};
    if d.paths.contains(Surface::Json) {
        Some(CallPath::Transport(KIT_CALLER.to_owned()))
    } else if d.paths.contains(Surface::Mcp) {
        Some(CallPath::Mcp)
    } else if d.paths.contains(Surface::Cli) {
        Some(CallPath::Cli)
    } else {
        None
    }
}

/// A fresh directory under the system temp dir, `op-registry-kit-<pid>-<n>`,
/// removed with everything in it when dropped. A kit runner makes one per run,
/// so sampled `Path` and `SecretFile` arguments never touch the crate.
pub(crate) struct KitDir {
    path: std::path::PathBuf,
}

impl KitDir {
    /// Creates the directory.
    ///
    /// # Panics
    ///
    /// Panics when the directory cannot be created.
    pub(crate) fn new() -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "op-registry-kit-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path)
            .unwrap_or_else(|e| panic!("the kit cannot create {}: {e}", path.display()));
        Self { path }
    }
}

impl Drop for KitDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The line `SecretFile` samples hold.
const SECRET_SAMPLE: &str = "op-registry-kit-sample\n";

/// `sampled` arguments of `d`, with each sampled `Path` field pointing at
/// `<dir>/<key>` and each sampled `SecretFile` field at a file in `dir` that
/// holds a fixed sample line. Keys the sample left out stay out.
pub(crate) fn kit_args(
    _d: &crate::Descriptor,
    fields: &[crate::FieldMeta],
    mut sampled: serde_json::Value,
    dir: &KitDir,
) -> serde_json::Value {
    use crate::FieldKind;
    let Some(map) = sampled.as_object_mut() else {
        return sampled;
    };
    for field in fields {
        if !matches!(field.kind, FieldKind::Path | FieldKind::SecretFile)
            || !map.get(&field.key).is_some_and(|v| v.is_string())
        {
            continue;
        }
        let file = dir.path.join(&field.key);
        if field.kind == FieldKind::SecretFile {
            std::fs::write(&file, SECRET_SAMPLE)
                .unwrap_or_else(|e| panic!("the kit cannot write {}: {e}", file.display()));
        }
        map.insert(
            field.key.clone(),
            serde_json::Value::String(file.to_string_lossy().into_owned()),
        );
    }
    sampled
}

/// The problem for an op whose sampled arguments a validator or parser
/// rejected: the first error of `errors` with an `op_registry.usage.*` code.
pub(crate) fn decode_problem(name: &str, errors: &[anyhow::Error]) -> Option<String> {
    errors.iter().find_map(|err| {
        let code = coded_error::facts_anyhow(err).code;
        code.starts_with("op_registry.usage.").then(|| {
            format!(
                "{name}: the sampled arguments did not decode ({code}: {err:#}); give a Fixture"
            )
        })
    })
}

/// The `FieldMeta` of op `d`: its first entry's parameter fields.
pub(crate) fn fields_of<'a, C>(
    registry: &'a crate::Registry<C>,
    d: &crate::Descriptor,
) -> &'a [crate::FieldMeta] {
    registry
        .entries
        .iter()
        .find(|e| e.descriptor.name == d.name)
        .map_or(&[], |e| e.param_fields.as_slice())
}

/// The line a runner prints, and adds to its report, for an op it cannot call.
pub(crate) fn typed_only_line(title: &str, d: &crate::Descriptor) -> String {
    format!("{title}: {}: typed only: not run", d.name)
}

/// Like [`fail_on`], and prints each of `skipped` to stderr; a report carries
/// them after its problems.
pub(crate) fn fail_on_noting(title: &str, problems: &[String], skipped: &[String]) {
    for line in skipped {
        eprintln!("{line}");
    }
    if problems.is_empty() {
        return;
    }
    let mut all = problems.to_vec();
    all.extend(skipped.iter().cloned());
    fail_on(title, &all);
}

/// Panics with `title` and one indented line per problem; does nothing when
/// there are none.
pub(crate) fn fail_on(title: &str, problems: &[String]) {
    if problems.is_empty() {
        return;
    }
    let mut report = format!("{title}: {} problem(s)\n", problems.len());
    for problem in problems {
        report.push_str("  - ");
        report.push_str(&problem.replace('\n', "\n    "));
        report.push('\n');
    }
    panic!("{report}");
}
