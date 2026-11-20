use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(one_of = "target")]
    id: Option<String>,
    name: Option<String>,
}

fn main() {}
