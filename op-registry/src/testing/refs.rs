//! `assert_unknown_ref_not_found`: a subject the app has never heard of is
//! `*.not_found`, never a value (spec 6.4).

use super::codes::call_errors;
use super::{fail_on_noting, kit_path, sample_params, typed_only_line, Fixture};
use crate::{Descriptor, FieldKind, Registry, Resolve};
use coded_error::ErrorClass;
use serde_json::Value;

/// Whether the op has a subject: a `Ref` field keyed `id`.
fn has_subject<C>(registry: &Registry<C>, d: &Descriptor) -> bool {
    registry
        .entries
        .iter()
        .filter(|e| e.descriptor.name == d.name)
        .any(|e| {
            e.param_fields
                .iter()
                .any(|f| f.kind == FieldKind::Ref && f.key == "id")
        })
}

/// A random 16-hex string, new on every call.
pub(crate) fn random_hex() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    format!("{:016x}", hasher.finish())
}

/// The arguments for `d`: the override's, else a sample.
pub(crate) fn args_for(d: &Descriptor, overrides: &[Fixture<'_>]) -> Value {
    overrides
        .iter()
        .find(|f| f.op == d.name)
        .map_or_else(|| sample_params(&d.input_schema), |f| f.args.clone())
}

/// Calls each op with a subject, on the first surface it declares, with `id` set to a random
/// 16-hex ref (the rest from `sample_params`, or the override for that op),
/// and expects `Err` of class Logical with a code ending `.not_found`.
///
/// # Panics
///
/// Panics with every op that answered otherwise.
pub async fn assert_unknown_ref_not_found<C: Resolve>(
    registry: &Registry<C>,
    ctx: &C,
    overrides: &[Fixture<'_>],
) {
    let mut problems = Vec::new();
    let mut skipped = Vec::new();
    for d in registry.descriptors() {
        if !has_subject(registry, d) {
            continue;
        }
        let Some(path) = kit_path(d) else {
            skipped.push(typed_only_line("assert_unknown_ref_not_found", d));
            continue;
        };
        let mut args = args_for(d, overrides);
        if !args.is_object() {
            args = Value::Object(Default::default());
        }
        let random = random_hex();
        args["id"] = Value::String(random.clone());
        let errors = match call_errors(registry, ctx, d, args, path)
            .await
            .errors_or_problem(&d.name)
        {
            Ok(errors) => errors,
            Err(problem) => {
                problems.push(problem);
                continue;
            }
        };
        let Some(err) = errors.first() else {
            problems.push(format!(
                "{}: the unknown ref `{random}` did not fail; a missing object is `not_found`, never a value",
                d.name
            ));
            continue;
        };
        let facts = coded_error::facts_anyhow(err);
        if facts.class != ErrorClass::Logical || !facts.code.ends_with(".not_found") {
            problems.push(format!(
                "{}: the unknown ref `{random}` failed with code `{}` (class {}), not a logical `.not_found`: {err:#}",
                d.name,
                facts.code,
                facts.class.as_str()
            ));
        }
    }
    fail_on_noting("assert_unknown_ref_not_found", &problems, &skipped);
}
