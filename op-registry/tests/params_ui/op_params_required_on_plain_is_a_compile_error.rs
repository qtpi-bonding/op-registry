use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(required)]
    x: String,
}

fn main() {}
