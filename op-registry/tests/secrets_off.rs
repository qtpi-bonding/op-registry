//! With the `secrets` feature off, a `SecretFile` field is a compile error;
//! with `prompt` off, a prompted field is never asked.

#[cfg(not(feature = "secrets"))]
#[test]
fn secret_file_needs_feature_is_a_compile_error() {
    trybuild::TestCases::new()
        .compile_fail("tests/secrets_ui/secret_file_needs_feature_is_a_compile_error.rs");
}

#[cfg(all(feature = "secrets", not(feature = "prompt")))]
mod no_prompt {
    use futures_util::future::BoxFuture;
    use op_registry::cli::{self, Io, NoHooks};
    use op_registry::{CallOptions, CliContext, OpGlobals, OpParams, OpSpec, Registry, SecretFile};
    use serde_json::{json, Value};

    #[derive(Debug, OpGlobals)]
    struct NoGlobals {}
    struct Ctx;
    impl op_registry::Resolve for Ctx {}
    impl CliContext for Ctx {
        type Globals = NoGlobals;
        fn from_globals(_g: &NoGlobals) -> anyhow::Result<Self> {
            Ok(Ctx)
        }
    }

    #[derive(Debug, OpParams)]
    struct Asked {
        #[op(prompt = "API key")]
        #[allow(dead_code)]
        key: SecretFile,
    }

    #[test]
    fn prompted_field_without_prompt_feature_is_missing() {
        let mut r: Registry<Ctx> = Registry::new();
        r.set_pillar("fix");
        r.push_op(
            OpSpec::<Asked, Value>::new("show", "Shows."),
            |_c: &Ctx, _p: Asked, o: CallOptions| -> BoxFuture<'_, anyhow::Result<Value>> {
                Box::pin(async move {
                    assert!(o.prompt_secret("x").is_none());
                    Ok(json!({}))
                })
            },
        );
        // `Io::std()` installs no prompter without the feature; memory io is
        // the same shape.
        let (io, cap) = Io::memory(b"", true, true);
        let code = futures_executor::block_on(cli::run_with_io(
            &r,
            vec!["app".into(), "show".into()],
            &NoHooks,
            io,
        ));
        assert_eq!(code, 2, "{}", cap.stderr());
        assert!(
            cap.stderr().contains("op_registry.usage.missing"),
            "{}",
            cap.stderr()
        );
        assert!(
            cap.stderr().contains("(no terminal to ask on)"),
            "{}",
            cap.stderr()
        );
        assert!(Io::std().prompter.is_none());
    }
}
