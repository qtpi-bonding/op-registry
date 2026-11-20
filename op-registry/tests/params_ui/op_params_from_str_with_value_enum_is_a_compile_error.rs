use clap::ValueEnum;
use op_registry::OpParams;

#[derive(Clone, ValueEnum)]
enum Mode {
    Fast,
}

#[derive(OpParams)]
struct Bad {
    #[op(from_str, value_enum)]
    x: Mode,
}

fn main() {}
