use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    #[op(at_least_one = "g")]
    x: Option<String>,
    #[op(at_least_one = "g")]
    y: Option<String>,
    #[op(one_of = "g")]
    z: Option<String>,
    #[op(one_of = "g")]
    w: Option<String>,
}

fn main() {}
