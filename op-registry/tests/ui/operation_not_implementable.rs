use op_registry::{Descriptor, OpParams, Operation};

struct X;

impl Operation for X {
    type Ctx = ();
    type Params = Params;
    type Output = String;

    fn descriptor() -> Descriptor {
        Descriptor::default()
    }
}

struct Params;

impl OpParams for Params {
    fn schema() -> serde_json::Value { serde_json::Value::Null }
    fn from_json(_: serde_json::Value) -> anyhow::Result<Self> { Ok(Self) }
    fn clap_args(cmd: clap::Command) -> clap::Command { cmd }
    fn from_matches(_: &clap::ArgMatches) -> anyhow::Result<Self> { Ok(Self) }
}

fn main() {}
