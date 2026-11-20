use op_registry::OpParams;

#[derive(OpParams)]
struct Common {
    z: Option<String>,
}

#[derive(OpParams)]
struct Bad {
    #[op(required_unless = "y..z")]
    x: Option<String>,
    #[op(flatten)]
    y: Common,
}

fn main() {}
