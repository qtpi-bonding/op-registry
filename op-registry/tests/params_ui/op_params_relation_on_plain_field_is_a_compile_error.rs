use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(one_of = "target")]
    id: Option<String>,
    #[op(one_of = "target")]
    x: String,
}

fn main() {}
