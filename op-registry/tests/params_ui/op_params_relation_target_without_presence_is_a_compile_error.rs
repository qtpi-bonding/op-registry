use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(required_unless = "y")]
    x: Option<String>,
    y: String,
}

fn main() {}
