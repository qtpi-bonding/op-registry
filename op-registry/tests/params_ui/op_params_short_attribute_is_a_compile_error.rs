use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(short = 'x')]
    value: String,
}

fn main() {}
