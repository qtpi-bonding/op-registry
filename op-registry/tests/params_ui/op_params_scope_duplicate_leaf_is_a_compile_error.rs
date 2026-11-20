use op_registry::{OpParams, Ref};

#[derive(OpParams)]
struct A {
    root: String,
}

#[derive(OpParams)]
struct B {
    root: String,
}

#[derive(OpParams)]
struct Bad {
    #[op(subject = "build", scope = ["a.root", "b.root"])]
    id: Ref,
    #[op(flatten)]
    a: A,
    #[op(flatten)]
    b: B,
}

fn main() {}
