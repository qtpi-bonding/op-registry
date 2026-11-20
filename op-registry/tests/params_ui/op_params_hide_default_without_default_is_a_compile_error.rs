use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(hide_default)]
    x: u32,
}

fn main() {}
