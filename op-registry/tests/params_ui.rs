#[test]
fn op_params_compile_failures() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/params_ui/op_params_unknown_attribute_key_is_a_compile_error.rs");
    tests.compile_fail(
        "tests/params_ui/op_params_required_unless_present_needs_option_is_a_compile_error.rs",
    );
    tests.compile_fail("tests/params_ui/op_params_unsupported_field_type_is_a_compile_error.rs");
}
