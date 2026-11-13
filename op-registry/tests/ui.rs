#[test]
fn operation_trait_is_sealed() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/operation_not_implementable.rs");
}
