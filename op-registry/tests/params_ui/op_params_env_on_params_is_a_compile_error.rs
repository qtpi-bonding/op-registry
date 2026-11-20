use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(env = "X")]
    value: String,
}

fn main() {}
