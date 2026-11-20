use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(sample = "7")]
    x: u32,
}

fn main() {}
