use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(conflicts_with = "y", required_unless = "y")]
    x: Option<String>,
    y: Option<String>,
}

fn main() {}
