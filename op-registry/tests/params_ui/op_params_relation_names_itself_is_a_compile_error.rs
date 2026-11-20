use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(conflicts_with = "x")]
    x: Option<String>,
}

fn main() {}
