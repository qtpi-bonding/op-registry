use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    value: std::path::PathBuf,
}

fn main() {}
