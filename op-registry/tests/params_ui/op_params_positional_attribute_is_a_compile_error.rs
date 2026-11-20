use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(positional)]
    value: String,
}

fn main() {}
