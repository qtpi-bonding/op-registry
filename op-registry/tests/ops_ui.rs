#[test]
fn ops_compile_failures() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ops_ui/ops_unknown_key_is_a_compile_error.rs");
    tests.compile_fail("tests/ops_ui/ops_missing_params_is_a_compile_error.rs");
    tests.compile_fail("tests/ops_ui/ops_no_kind_is_a_compile_error.rs");
    tests.compile_fail("tests/ops_ui/ops_two_kinds_is_a_compile_error.rs");
    tests.compile_fail("tests/ops_ui/ops_item_without_list_is_a_compile_error.rs");
    tests.compile_fail("tests/ops_ui/ops_output_with_list_is_a_compile_error.rs");
    tests.compile_fail("tests/ops_ui/ops_exit_out_of_range_is_a_compile_error.rs");
    tests.compile_fail("tests/ops_ui/ops_cursor_on_non_stream_is_a_compile_error.rs");
    tests.compile_fail("tests/ops_ui/ops_default_on_one_word_path_is_a_compile_error.rs");
    tests.compile_fail("tests/ops_ui/ops_text_on_list_is_a_compile_error.rs");
    tests.compile_fail("tests/ops_ui/ops_exits_on_list_is_a_compile_error.rs");
    tests.compile_fail("tests/ops_ui/ops_exits_without_ok_exit_is_a_compile_error.rs");
    tests.compile_fail("tests/ops_ui/ops_globals_mismatch_is_a_compile_error.rs");
    tests.compile_fail("tests/ops_ui/ops_paths_unknown_word_is_a_compile_error.rs");
    tests.compile_fail("tests/ops_ui/ops_paths_empty_is_a_compile_error.rs");
    tests.compile_fail("tests/ops_ui/ops_paths_duplicate_is_a_compile_error.rs");
    tests.compile_fail("tests/ops_ui/ops_reason_key_is_a_compile_error.rs");
}
