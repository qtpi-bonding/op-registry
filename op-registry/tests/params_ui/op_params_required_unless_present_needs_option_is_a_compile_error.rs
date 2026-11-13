use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(required_unless_present = "other")]
    value: String,
    other: Option<String>,
}

fn main() {}
