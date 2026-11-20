use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(conflicts_with = "y")]
    x: Option<String>,
}

fn main() {}
