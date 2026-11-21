use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(prompt)]
    x: String,
}

fn main() {}
