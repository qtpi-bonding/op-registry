#![allow(dead_code, unused_imports)]
use op_registry::{CallOptions, OpParams};

pub struct Ctx;
impl op_registry::Resolve for Ctx {}

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "fix.err")]
pub enum E {
    #[error("bad")]
    #[coded(code = "bad", class = Logical)]
    Bad,
}

#[derive(OpParams)]
pub struct P {
    pub v: Option<String>,
}

#[derive(serde::Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct O {
    pub ok: bool,
}

async fn f(_c: &Ctx, _p: P, _o: CallOptions) -> anyhow::Result<O> {
    Ok(O { ok: true })
}

op_registry::ops! {
    Ctx = Ctx;
    errors = [E];
    errors = [E];
    /// Doc.
    "a b" => { params: P, output: O, access: reads, audience: agent, op: f },
}

fn main() {}
