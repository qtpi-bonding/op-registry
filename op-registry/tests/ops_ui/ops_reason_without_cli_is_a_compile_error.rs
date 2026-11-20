#![allow(dead_code, unused_imports)]
use op_registry::{CallOptions, OpParams};

pub struct Ctx;
impl op_registry::Resolve for Ctx {}

#[derive(OpParams)]
pub struct P {
    pub v: Option<String>,
}

pub struct O {
    pub ok: bool,
}

async fn f(_c: &Ctx, _p: P, _o: CallOptions) -> anyhow::Result<O> {
    Ok(O { ok: true })
}

op_registry::ops! {
    Ctx = Ctx;
    /// Doc.
    "a b" => { params: P, output: O, access: reads, audience: owner, op: f, reason: "no path" },
}

fn main() {}
