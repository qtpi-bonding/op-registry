use op_registry::{OpParams, Ref};

#[derive(OpParams)]
struct Bad {
    #[op(subject = "build")]
    id: Ref,
    #[op(kind = "agent", absent_ok)]
    other: Option<Ref>,
}

fn main() {}
