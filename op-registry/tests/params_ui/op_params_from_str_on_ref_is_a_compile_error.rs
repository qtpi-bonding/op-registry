use op_registry::{OpParams, Ref};

#[derive(OpParams)]
struct Bad {
    #[op(kind = "agent", from_str)]
    x: Ref,
}

fn main() {}
