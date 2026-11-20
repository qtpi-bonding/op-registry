use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(one_of = "g")]
    id: Option<String>,
    name: Option<String>,
}

fn main() {}
