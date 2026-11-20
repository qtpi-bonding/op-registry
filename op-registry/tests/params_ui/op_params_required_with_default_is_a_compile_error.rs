use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(required, default = "a")]
    x: Option<String>,
}

fn main() {}
