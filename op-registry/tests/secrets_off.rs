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
    use op_registry::cli::{self, Io, NoHooks, Prompter};
    use op_registry::{CallOptions, CliContext, OpGlobals, OpParams, OpSpec, Registry, SecretFile};
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};

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

    struct Fixed;
    impl Prompter for Fixed {
        fn read_secret(&self, _label: &str) -> std::io::Result<zeroize::Zeroizing<String>> {
            Ok(zeroize::Zeroizing::new("typed-by-a-host".to_owned()))
        }
    }

    #[derive(Debug, OpParams)]
    struct Plain {
        /// Unused.
        #[allow(dead_code)]
        note: Option<String>,
    }

    #[derive(Debug, OpParams)]
    struct Asked {
        #[op(prompt = "API key")]
        #[allow(dead_code)]
        key: SecretFile,
    }

    /// Runs the prompted op and a plain op that records `prompt_secret`.
    fn run(args: &[&str], io: Io, seen: &Arc<Mutex<Option<bool>>>, ran: &Arc<Mutex<bool>>) -> u8 {
        let mut r: Registry<Ctx> = Registry::new();
        r.set_pillar("fix");
        let ran = ran.clone();
        r.push_op(
            OpSpec::<Asked, Value>::new("show", "Shows."),
            move |_c: &Ctx, _p: Asked, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Value>> {
                *ran.lock().unwrap() = true;
                Box::pin(async move { Ok(json!({})) })
            },
        );
        let seen = seen.clone();
        r.push_op(
            OpSpec::<Plain, Value>::new("ask", "Asks."),
            move |_c: &Ctx, _p: Plain, o: CallOptions| -> BoxFuture<'_, anyhow::Result<Value>> {
                *seen.lock().unwrap() = Some(o.prompt_secret("x").is_some());
                Box::pin(async move { Ok(json!({})) })
            },
        );
        let argv = std::iter::once("app")
            .chain(args.iter().copied())
            .map(Into::into)
            .collect();
        futures_executor::block_on(cli::run_with_io(&r, argv, &NoHooks, io))
    }

    #[test]
    fn prompted_field_without_prompt_feature_is_missing() {
        let (seen, ran) = (Arc::default(), Arc::default());
        // `Io::std()` installs no prompter without the feature.
        assert!(Io::std().prompter.is_none());
        let (io, cap) = Io::memory(b"", true, true);
        assert_eq!(run(&["show"], io, &seen, &ran), 2, "{}", cap.stderr());
        let err = cap.stderr();
        assert!(err.contains("op_registry.usage.missing"), "{err}");
        assert!(err.contains("(no terminal to ask on)"), "{err}");
        assert!(!*ran.lock().unwrap(), "the prompted op must not run");
        // a plain op runs and gets None
        let (io, cap) = Io::memory(b"", true, true);
        assert_eq!(run(&["ask"], io, &seen, &ran), 0, "{}", cap.stderr());
        assert_eq!(*seen.lock().unwrap(), Some(false));
    }

    #[test]
    fn prompt_secret_is_none_even_with_an_installed_prompter() {
        let (seen, ran) = (Arc::default(), Arc::default());
        let (io, cap) = Io::memory(b"", true, true);
        let io = io.with_prompter(Arc::new(Fixed));
        assert_eq!(run(&["ask"], io, &seen, &ran), 0, "{}", cap.stderr());
        assert_eq!(*seen.lock().unwrap(), Some(false));
        // prompted-field behaviour is unchanged: it still asks the installed prompter
        let (io, cap) = Io::memory(b"", true, true);
        let io = io.with_prompter(Arc::new(Fixed));
        assert_eq!(run(&["show"], io, &seen, &ran), 0, "{}", cap.stderr());
        assert!(*ran.lock().unwrap());
    }
}
