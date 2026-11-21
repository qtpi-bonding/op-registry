use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(prompt)]
    x: Option<op_registry::SecretFile>,
}

fn main() {}
