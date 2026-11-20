use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(at_least_one = "g")]
    x: Option<String>,
    y: Option<String>,
}

fn main() {}
