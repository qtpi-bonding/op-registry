use op_registry::{OpParams, Ref};

#[derive(OpParams)]
struct Bad {
    #[op(subject = "build")]
    id: Ref,
    #[op(subject = "agent")]
    other: Ref,
}

fn main() {}
