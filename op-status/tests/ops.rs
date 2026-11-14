#![allow(clippy::err_expect, clippy::ok_expect)]
use futures_util::future::BoxFuture;
use op_registry::cli::{self, CliOutput};
use op_registry::parity::{assert_parity, cli_surface};
use op_registry::{mcp, registry, Audience, CallOptions, OpParams, Registry};
use op_status::ops::{status_registry, StatusOptions};
use op_status::{Component, ServiceInfo, State, StatusFilter, StatusSource};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};

const NOW: i64 = 1_700_000_000_123;

struct ToySource {
    components: Vec<Component>,
    allowed: &'static [&'static str],
    last_max: AtomicUsize,
}

impl ToySource {
    fn new(components: Vec<Component>) -> Self {
        Self {
            components,
            allowed: &["count", "name"],
            last_max: AtomicUsize::new(usize::MAX),
        }
    }
}

impl StatusSource for ToySource {
    fn service(&self) -> ServiceInfo {
        ServiceInfo {
            name: "toy".into(),
            version: "0.9.1".into(),
        }
    }
    fn epoch(&self) -> u64 {
        7
    }
    fn now_unix_ms(&self) -> i64 {
        NOW
    }
    fn snapshot<'a>(
        &'a self,
        filter: &'a StatusFilter,
        max: usize,
    ) -> BoxFuture<'a, anyhow::Result<Vec<Component>>> {
        self.last_max.store(max, Ordering::SeqCst);
        Box::pin(async move {
            Ok(self
                .components
                .iter()
                .filter(|c| filter.kind.as_ref().is_none_or(|k| &c.kind == k))
                .filter(|c| filter.id.as_ref().is_none_or(|i| &c.id == i))
                .take(max + 1)
                .cloned()
                .collect())
        })
    }
    fn allowed_attr_keys(&self) -> &'static [&'static str] {
        self.allowed
    }
}

fn many(n: usize) -> Vec<Component> {
    (0..n)
        .map(|i| Component::new("room", format!("r{i:02}"), State::Ok))
        .collect()
}

fn call(reg: &Registry<ToySource>, src: &ToySource, args: Value) -> anyhow::Result<Value> {
    futures_executor::block_on(reg.call("status_get", args, src, CallOptions::default()))
}

fn ids(v: &Value) -> Vec<String> {
    v["components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn status_get_filters_by_kind_and_id() {
    let src = ToySource::new(vec![
        Component::new("room", "a", State::Ok),
        Component::new("room", "b", State::Down),
        Component::new("node", "a", State::Ok),
    ]);
    let reg = status_registry::<ToySource>(StatusOptions::new());
    let all = call(&reg, &src, json!({})).unwrap();
    assert_eq!(ids(&all), vec!["a", "b", "a"]);
    let rooms = call(&reg, &src, json!({"kind": "room"})).unwrap();
    assert_eq!(ids(&rooms), vec!["a", "b"]);
    let one = call(&reg, &src, json!({"kind": "room", "id": "b"})).unwrap();
    assert_eq!(ids(&one), vec!["b"]);
    assert_eq!(one["components"][0]["state"], "DOWN");
    let a = call(&reg, &src, json!({"id": "a"})).unwrap();
    assert_eq!(a["components"].as_array().unwrap().len(), 2);
    let none = call(&reg, &src, json!({"kind": "nothing"})).unwrap();
    assert_eq!(none["components"], json!([]));
    assert_eq!(none["truncated"], false);
}

#[test]
fn status_get_marks_truncated_and_never_returns_a_partial_without_it() {
    let src = ToySource::new(many(10));
    let reg = status_registry::<ToySource>(StatusOptions::new().max_components(5));

    // limit below the number of rows: truncated, extra row dropped.
    let v = call(&reg, &src, json!({"limit": 3})).unwrap();
    assert_eq!(ids(&v), vec!["r00", "r01", "r02"]);
    assert_eq!(v["truncated"], true);
    assert_eq!(src.last_max.load(Ordering::SeqCst), 3);

    // Exactly `limit` rows available: not truncated.
    let src4 = ToySource::new(many(4));
    let v = call(&reg, &src4, json!({"limit": 4})).unwrap();
    assert_eq!(v["components"].as_array().unwrap().len(), 4);
    assert_eq!(v["truncated"], false);

    // No limit: defaults to max_components.
    let v = call(&reg, &src, json!({})).unwrap();
    assert_eq!(v["components"].as_array().unwrap().len(), 5);
    assert_eq!(v["truncated"], true);

    // limit above max_components is clamped.
    let v = call(&reg, &src, json!({"limit": 100})).unwrap();
    assert_eq!(v["components"].as_array().unwrap().len(), 5);
    assert_eq!(v["truncated"], true);
    assert_eq!(src.last_max.load(Ordering::SeqCst), 5);

    // Everything fits: not truncated.
    let small = ToySource::new(many(2));
    let v = call(&reg, &small, json!({"limit": 100})).unwrap();
    assert_eq!(v["components"].as_array().unwrap().len(), 2);
    assert_eq!(v["truncated"], false);

    // Invariant: a short result never claims to be complete.
    for limit in 1..=10u32 {
        let v = call(&reg, &src, json!({"limit": limit})).unwrap();
        let n = v["components"].as_array().unwrap().len();
        if n < 10 {
            assert_eq!(v["truncated"], true, "limit {limit}");
        }
    }
}

fn leaky_source() -> ToySource {
    ToySource::new(vec![Component::new("room", "r1", State::Ok)
        .attr("count", 2i64)
        .attr("name", "alpha")
        .attr("leaked_path", "/home/x")])
}

#[cfg(debug_assertions)]
#[test]
fn status_get_refuses_unlisted_attr_in_debug_and_drops_it_in_release_debug() {
    let reg = status_registry::<ToySource>(StatusOptions::new());
    let err = call(&reg, &leaky_source(), json!({}))
        .err()
        .expect("must fail");
    let facts = coded_error::facts_anyhow(&err);
    assert_eq!(facts.code, "op_status.attr_not_allowed");
    assert_eq!(facts.class, coded_error::ErrorClass::Logical);
    assert!(format!("{err:#}").contains("leaked_path"));
}

#[cfg(not(debug_assertions))]
#[test]
fn status_get_refuses_unlisted_attr_in_debug_and_drops_it_in_release_release() {
    let reg = status_registry::<ToySource>(StatusOptions::new());
    let v = call(&reg, &leaky_source(), json!({})).unwrap();
    assert_eq!(
        v["components"][0]["attrs"],
        json!({"count": 2, "name": "alpha"})
    );
    assert!(!v.to_string().contains("leaked_path"));
    assert!(!v.to_string().contains("/home/x"));
}

#[test]
fn status_ops_use_the_audience_parameter() {
    let default_reg = status_registry::<ToySource>(StatusOptions::new());
    assert_eq!(
        default_reg.get("status_get").unwrap().audience,
        Audience::Agent
    );
    assert!(mcp::tool_list(&default_reg, Audience::Agent)
        .iter()
        .any(|t| t.name == "status_get"));
    assert!(mcp::tool_list(&default_reg, Audience::Owner).is_empty());

    let owner = status_registry::<ToySource>(StatusOptions::new().audience(Audience::Owner));
    assert_eq!(owner.get("status_get").unwrap().audience, Audience::Owner);
    assert!(mcp::tool_list(&owner, Audience::Agent)
        .iter()
        .all(|t| t.name != "status_get"));
    assert!(mcp::tool_list(&owner, Audience::Owner)
        .iter()
        .any(|t| t.name == "status_get"));
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, OpParams)]
struct Empty {
    #[allow(dead_code)]
    value: Option<i32>,
}

async fn ping(_: &ToySource, _: Empty) -> anyhow::Result<Value> {
    Ok(json!({"ok": true}))
}

#[test]
fn status_registry_merges_into_an_app_registry_and_cli_and_mcp_parity_holds() {
    let mut app: Registry<ToySource> = registry! {
        Ctx = ToySource;
        entries: [
            {
                name: "ping", noun: "system", verb: "ping", description: "pings",
                access: none, audience: Agent, params: Empty, output: Value,
                op: ping,
            },
        ]
    };
    app.merge(status_registry::<ToySource>(StatusOptions::new()));
    assert_eq!(app.names(), vec!["ping", "status_get"]);

    let cmd = cli::command(&app, "app", "toy app");
    cmd.clone().debug_assert();
    let surface = cli_surface(&cmd);
    let tools = mcp::tool_list(&app, Audience::Agent);
    assert_parity(&app, &surface, &tools, &[], &[], Audience::Agent).unwrap();
    assert!(surface.names.contains("status_get"));
    assert!(surface.names.contains("ping"));
    assert!(tools.iter().any(|t| t.name == "status_get"));

    // CLI flags --kind --id --limit reach the op and match the MCP result.
    let src = ToySource::new(many(5));
    let matches = cmd
        .try_get_matches_from(["app", "status", "get", "--kind", "room", "--limit", "2"])
        .unwrap();
    let from_cli =
        futures_executor::block_on(cli::dispatch(&app, &src, &matches, CallOptions::default()))
            .ok()
            .expect("dispatch");
    let CliOutput::Value(from_cli) = from_cli else {
        panic!("expected a value");
    };
    let from_mcp = futures_executor::block_on(mcp::call_tool(
        &app,
        &src,
        Audience::Agent,
        "status_get",
        json!({"kind": "room", "limit": 2}),
        CallOptions::default(),
    ))
    .ok()
    .expect("call_tool");
    assert_eq!(from_cli, from_mcp);
    assert_eq!(from_cli["truncated"], true);

    let status = app
        .descriptors()
        .into_iter()
        .find(|d| d.name == "status_get")
        .unwrap();
    assert_eq!(
        (status.noun.as_str(), status.verb.as_str()),
        ("status", "get")
    );
    assert!(!status.streaming);
}

#[test]
fn status_get_json_shape_is_pinned() {
    let src = ToySource::new(vec![
        Component::new("room", "r1", State::Degraded)
            .parent("service", "main")
            .ready(true)
            .last_error("net.timeout", "timed out")
            .consecutive_failures(3)
            .changed_at_unix_ms(1_699_999_999_000)
            .attr("count", 2i64)
            .attr("name", "alpha"),
        Component::new("node", "n1", State::Stopped).terminal(true),
    ]);
    let reg = status_registry::<ToySource>(StatusOptions::new());
    let v = call(&reg, &src, json!({})).unwrap();
    let expected = json!({
        "service": "toy",
        "version": "0.9.1",
        "taken_at_unix_ms": NOW,
        "epoch": 7,
        "components": [
            {
                "kind": "room", "id": "r1",
                "parent": {"kind": "service", "id": "main"},
                "state": "DEGRADED", "ready": true, "terminal": false,
                "last_error_code": "net.timeout",
                "last_error_message": "timed out",
                "consecutive_failures": 3,
                "changed_at_unix_ms": 1_699_999_999_000i64,
                "attrs": {"count": 2, "name": "alpha"}
            },
            {
                "kind": "node", "id": "n1", "parent": null,
                "state": "STOPPED", "ready": false, "terminal": true,
                "last_error_code": "", "last_error_message": "",
                "consecutive_failures": null, "changed_at_unix_ms": 0,
                "attrs": {}
            }
        ],
        "truncated": false
    });
    assert_eq!(v, expected);
}
