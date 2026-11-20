//! With the `secrets` feature off, a `SecretFile` field is a compile error.
#![cfg(not(feature = "secrets"))]

#[test]
fn secret_file_needs_feature_is_a_compile_error() {
    trybuild::TestCases::new()
        .compile_fail("tests/secrets_ui/secret_file_needs_feature_is_a_compile_error.rs");
}
