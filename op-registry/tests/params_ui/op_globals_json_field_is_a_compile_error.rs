use op_registry::OpGlobals;

#[derive(OpGlobals)]
struct Bad {
    value: op_registry::Json,
}

fn main() {}
