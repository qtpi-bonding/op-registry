#[test]
fn op_params_compile_failures() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/params_ui/op_params_unknown_attribute_key_is_a_compile_error.rs");
    tests.compile_fail("tests/params_ui/op_params_unsupported_field_type_is_a_compile_error.rs");
    tests.compile_fail("tests/params_ui/op_params_tuple_struct_is_a_compile_error.rs");
    tests.compile_fail("tests/params_ui/op_params_long_attribute_is_a_compile_error.rs");
    tests.compile_fail("tests/params_ui/op_params_short_attribute_is_a_compile_error.rs");
    tests.compile_fail("tests/params_ui/op_params_positional_attribute_is_a_compile_error.rs");
    tests.compile_fail("tests/params_ui/op_params_env_on_params_is_a_compile_error.rs");
    tests.compile_fail("tests/params_ui/op_params_one_of_needs_two_options_is_a_compile_error.rs");
    tests.compile_fail("tests/params_ui/op_params_clearable_needs_option_is_a_compile_error.rs");
    tests.compile_fail("tests/params_ui/op_params_two_subjects_is_a_compile_error.rs");
    tests.compile_fail("tests/params_ui/op_params_scope_names_unknown_field_is_a_compile_error.rs");
    tests.compile_fail("tests/params_ui/op_params_secret_inline_needs_secret_file_is_a_compile_error.rs");
}
