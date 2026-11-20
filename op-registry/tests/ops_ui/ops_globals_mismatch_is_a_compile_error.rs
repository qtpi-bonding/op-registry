#![allow(dead_code, unused_imports)]
use op_registry::{CallOptions, CliContext, OpGlobals, OpParams};

pub struct Ctx;
impl op_registry::Resolve for Ctx {}

#[derive(OpGlobals)]
pub struct Globals {
    pub hub: Option<String>,
}

#[derive(OpGlobals)]
pub struct OtherGlobals {
    pub hub: Option<String>,
}

impl CliContext for Ctx {
    type Globals = Globals;
    fn from_globals(_g: &Globals) -> anyhow::Result<Self> {
        Ok(Ctx)
    }
}

#[derive(OpParams)]
pub struct P {
    pub v: Option<String>,
}

async fn f(_c: &Ctx, _p: P, _o: CallOptions) -> anyhow::Result<serde_json::Value> {
    Ok(serde_json::Value::Null)
}

op_registry::ops! {
    Ctx = Ctx;
    globals = OtherGlobals;
    /// Doc.
    "a b" => { params: P, output: serde_json::Value, access: reads, audience: agent, op: f },
}

fn main() {}
