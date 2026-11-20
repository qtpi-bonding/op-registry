use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(clearable)]
    host: Option<String>,
}

fn main() {}
