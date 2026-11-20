use op_registry::OpParams;

#[derive(OpParams)]
#[op(validate)]
struct Bad {
    value: String,
}

fn main() {}
