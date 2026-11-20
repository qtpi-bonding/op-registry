use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(ignore_case)]
    x: String,
}

fn main() {}
