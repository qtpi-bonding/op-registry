use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(allow_hyphen)]
    x: bool,
}

fn main() {}
