use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(required)]
    x: bool,
}

fn main() {}
