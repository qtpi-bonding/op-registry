//! `assert_unknown_ref_not_found`: a subject the app has never heard of is
//! `*.not_found`, never a value (spec 6.4).

use super::codes::call_errors;
use super::{
    decode_problem, fail_on_noting, fields_of, kit_args, kit_path, sample_params_seeded,
    typed_only_line, Fixture, KitDir,
};
use crate::{Descriptor, Registry, Resolve};
use coded_error::ErrorClass;
use serde_json::{Map, Value};

/// The subject field of `d` the kit tests: one whose subject is not `absent_ok`.
fn tested_subject<'a, C>(
    registry: &'a Registry<C>,
    d: &Descriptor,
) -> Option<&'a crate::FieldMeta> {
    super::fields_of(registry, d)
        .iter()
        .find(|f| f.subject.as_ref().is_some_and(|s| !s.absent_ok))
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

/// The arguments for `d`: the override's, else a sample (seeded) made
/// to decode, with paths in `dir`. The flag says it was sampled.
pub(crate) fn args_for<C>(
    registry: &Registry<C>,
    d: &Descriptor,
    overrides: &[Fixture<'_>],
    seed: Map<String, Value>,
    dir: &KitDir,
) -> (Value, bool) {
    match overrides.iter().find(|f| f.op == d.name) {
        Some(f) => (f.args.clone(), false),
        None => {
            let sampled = sample_params_seeded(&d.input_schema, seed);
            (kit_args(d, fields_of(registry, d), sampled, dir), true)
        }
    }
}

/// Calls each op with a subject that is not `absent_ok`, on the first surface it declares, with `id` set to a random
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
    let dir = KitDir::new();
    for d in registry.descriptors() {
        let Some(subject) = tested_subject(registry, d) else {
            continue;
        };
        let Some(path) = kit_path(d) else {
            skipped.push(typed_only_line("assert_unknown_ref_not_found", d));
            continue;
        };
        let random = random_hex();
        let seed = Map::from_iter([(subject.key.clone(), Value::String(random.clone()))]);
        let (mut args, sampled) = args_for(registry, d, overrides, seed, &dir);
        if !args.is_object() {
            args = Value::Object(Default::default());
        }
        args[subject.key.as_str()] = Value::String(random.clone());
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
        if sampled {
            if let Some(problem) = decode_problem(&d.name, &errors) {
                problems.push(problem);
                continue;
            }
        }
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
