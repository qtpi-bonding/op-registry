use op_registry::{OpParams, Ref};

#[derive(OpParams)]
struct Group {
    root: String,
}

#[derive(OpParams)]
struct Bad {
    #[op(subject = "build", scope = ["nope.root"])]
    id: Ref,
    #[op(flatten)]
    project: Group,
}

fn main() {}
