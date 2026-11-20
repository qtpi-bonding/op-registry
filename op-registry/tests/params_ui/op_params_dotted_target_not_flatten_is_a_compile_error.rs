use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(required_unless = "y.z")]
    x: Option<String>,
    y: Option<String>,
}

fn main() {}
