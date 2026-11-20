use op_registry::OpParams;

#[derive(OpParams)]
#[op(mystery = "value")]
struct Bad {
    value: String,
}

fn main() {}
