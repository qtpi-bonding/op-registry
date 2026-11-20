//! `paths: [cli]`: an op on the CLI and the typed path, never on a JSON path.

use futures_util::future::BoxFuture;
use op_registry::cli::{self, Io, NoHooks};
use op_registry::mcp::{call_tool, tool_list};
use op_registry::testing::{assert_parity, parity_report, Mount, Reach};
use op_registry::{
    Audience, CallOptions, CliContext, OpGlobals, OpParams, OpSpec, Paths, Registry,
};
use serde::Serialize;
use serde_json::json;
use std::ffi::OsString;

#[derive(OpGlobals)]
pub struct NoGlobals {}

pub struct Ctx;
impl op_registry::Resolve for Ctx {}
impl CliContext for Ctx {
    type Globals = NoGlobals;
    fn from_globals(_: &NoGlobals) -> anyhow::Result<Self> {
        Ok(Ctx)
    }
}

#[derive(Clone, OpParams)]
pub struct Nothing {}

#[derive(Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Out {
    pub signed: bool,
}

fn sign(_c: &Ctx, _p: Nothing, _o: CallOptions) -> BoxFuture<'_, anyhow::Result<Out>> {
    Box::pin(async { Ok(Out { signed: true }) })
}

const REASON: &str = "signs with the owner key";

fn registry() -> Registry<Ctx> {
    let mut r = Registry::<Ctx>::new();
    r.set_bin("tool", "1.0.0", "A fixture.");
    r.group("owner", "Owner commands.");
    let mut spec = OpSpec::<Nothing, Out>::new("owner sign", "Sign something.");
    spec.access = op_registry::Access::reads();
    spec.paths = Paths::CliOnly;
    spec.cli_only_reason = REASON;
    r.push_op(spec, sign);
    let mut plain = OpSpec::<Nothing, Out>::new("owner ping", "Ping.");
    plain.access = op_registry::Access::reads();
    r.push_op(plain, sign);
    r
}

fn argv(args: &[&str]) -> Vec<OsString> {
    std::iter::once("tool")
        .chain(args.iter().copied())
        .map(OsString::from)
        .collect()
}

fn wrong_path(err: &anyhow::Error) {
    assert_eq!(
        coded_error::facts_anyhow(err).code,
        "op_registry.call.wrong_path"
    );
}

#[test]
fn descriptor_carries_the_reason() {
    let r = registry();
    let d = r.get("owner_sign").unwrap();
    assert_eq!(d.paths, Paths::CliOnly);
    assert_eq!(d.cli_only_reason, REASON);
    assert_eq!(r.get("owner_ping").unwrap().cli_only_reason, "");
}

#[test]
#[should_panic(expected = "needs a `cli_only_reason`")]
fn push_api_without_reason_panics() {
    let mut r = Registry::<Ctx>::new();
    r.group("owner", "Owner commands.");
    let mut spec = OpSpec::<Nothing, Out>::new("owner sign", "Sign.");
    spec.paths = Paths::CliOnly;
    r.push_op(spec, sign);
}

#[test]
fn tool_list_excludes_it_for_every_audience() {
    let r = registry();
    for audience in [Audience::Agent, Audience::Owner, Audience::Dev] {
        let names: Vec<String> = tool_list(&r, audience)
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert!(!names.contains(&"owner_sign".to_owned()), "{names:?}");
    }
    let agent: Vec<String> = tool_list(&r, Audience::Agent)
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(agent, vec!["owner_ping"]);
}

#[test]
fn call_tool_rejects_it_with_the_coded_error() {
    let r = registry();
    let err = futures_executor::block_on(call_tool(
        &r,
        &Ctx,
        Audience::Agent,
        "owner_sign",
        json!({}),
        CallOptions::default(),
    ))
    .unwrap_err();
    wrong_path(err.error());
}

#[test]
fn call_rejects_it_with_the_coded_error() {
    let r = registry();
    let err =
        futures_executor::block_on(r.call("owner_sign", json!({}), &Ctx, CallOptions::default()))
            .unwrap_err();
    wrong_path(err.error());
}

#[test]
fn the_cli_runs_it() {
    let (io, cap) = Io::memory(b"", false, false);
    let code = futures_executor::block_on(cli::run_with_io(
        &registry(),
        argv(&["owner", "sign"]),
        &NoHooks,
        io,
    ));
    assert_eq!(code, 0, "{}", cap.stderr());
    assert_eq!(cap.stdout(), "{\"signed\":true}\n");
}

mod declared {
    use super::*;

    async fn sign_it(_c: &Ctx, _p: Nothing, _o: CallOptions) -> anyhow::Result<Out> {
        Ok(Out { signed: true })
    }

    op_registry::ops! {
        Ctx = Ctx;
        globals = NoGlobals;
        groups = { "owner" => "Owner commands." };
        /// Sign something.
        "owner sign" => { params: Nothing, output: Out, access: reads, audience: owner,
                          op: sign_it, paths: [cli], reason: "signs with the owner key" },
    }

    pub use self::{ops::OwnerSign, registry as declared_registry};
}

#[test]
fn ops_macro_declares_it_and_the_typed_path_runs_it() {
    let r = declared::declared_registry();
    let d = r.get("owner_sign").unwrap();
    assert_eq!(d.paths, Paths::CliOnly);
    assert_eq!(d.cli_only_reason, REASON);
    let out = futures_executor::block_on(r.call_typed::<declared::OwnerSign>(
        &Ctx,
        Nothing {},
        CallOptions::default(),
    ))
    .unwrap();
    assert!(out.signed);
    assert!(tool_list(&r, Audience::Owner).is_empty());
}

#[test]
fn parity_passes_and_names_the_reason() {
    let r = registry();
    let mounts = [Mount {
        name: "rest",
        audience: Audience::Agent,
        ops: &["owner_ping"],
    }];
    assert_parity(&r, &mounts);
    let lines = parity_report(&r, &mounts);
    let find = |path: &str| {
        lines
            .iter()
            .find(|l| l.name == "owner_sign" && l.path == path)
            .map(|l| l.reach.clone())
    };
    assert_eq!(find("cli"), Some(Reach::Reachable));
    assert_eq!(
        find("mcp"),
        Some(Reach::Absent {
            reason: format!("cli only: {REASON}")
        })
    );
    assert_eq!(find("rest"), None);
}

fn panic_text(f: impl FnOnce()) -> String {
    let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_err();
    err.downcast_ref::<String>().cloned().unwrap_or_default()
}

#[test]
fn parity_fails_when_a_cli_only_op_is_in_a_mount() {
    let r = registry();
    let mounts = [Mount {
        name: "rest",
        audience: Audience::Agent,
        ops: &["owner_ping", "owner_sign"],
    }];
    let text = panic_text(|| assert_parity(&r, &mounts));
    assert!(
        text.contains("cli-only op owner_sign is in mount rest"),
        "{text}"
    );
}
