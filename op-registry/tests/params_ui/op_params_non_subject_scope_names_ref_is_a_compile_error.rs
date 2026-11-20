use op_registry::{OpParams, Ref};

#[derive(OpParams)]
struct Bad {
    #[op(subject = "build")]
    id: Ref,
    #[op(kind = "blueprint")]
    blueprint: Ref,
    #[op(kind = "collection", scope = ["blueprint"])]
    collection: Option<Ref>,
}

fn main() {}
