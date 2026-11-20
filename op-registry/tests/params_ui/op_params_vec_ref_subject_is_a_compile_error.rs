use op_registry::{OpParams, Ref};

#[derive(OpParams)]
struct Bad {
    #[op(subject = "build")]
    id: Vec<Ref>,
}

fn main() {}
