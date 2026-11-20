use op_registry::OpParams;

#[derive(OpParams)]
struct Bad {
    value: std::net::IpAddr,
}

fn main() {}
