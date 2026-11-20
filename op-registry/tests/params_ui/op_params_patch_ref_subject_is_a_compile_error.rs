use op_registry::{OpParams, Patch, Ref};

#[derive(OpParams)]
struct Bad {
    #[op(subject = "build", clearable)]
    id: Patch<Ref>,
}

fn main() {}
