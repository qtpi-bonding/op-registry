use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(mystery = "value")]
    value: String,
}

fn main() {}
