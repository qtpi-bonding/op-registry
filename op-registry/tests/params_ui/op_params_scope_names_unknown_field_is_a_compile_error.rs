use op_registry::{OpParams, Ref};

#[derive(OpParams)]
struct Bad {
    #[op(subject = "build", scope = ["blueprint"])]
    id: Ref,
}

fn main() {}
