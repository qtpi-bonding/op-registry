use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(env = "X")]
    x: Vec<String>,
}

fn main() {}
