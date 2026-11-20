use op_registry::OpParams;

#[derive(OpParams)]
struct Params {
    #[op(secret_inline)]
    name: String,
}

fn main() {}
