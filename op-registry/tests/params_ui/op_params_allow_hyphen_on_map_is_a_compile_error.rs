use op_registry::OpParams;
use std::collections::BTreeMap;

#[derive(OpParams)]
struct Bad {
    #[op(allow_hyphen)]
    x: BTreeMap<String, String>,
}

fn main() {}
