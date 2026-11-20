use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(long = "x")]
    value: String,
}

fn main() {}
