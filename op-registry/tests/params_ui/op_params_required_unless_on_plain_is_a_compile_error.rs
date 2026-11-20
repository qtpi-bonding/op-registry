use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(required_unless = "y")]
    x: String,
    y: Option<String>,
}

fn main() {}
