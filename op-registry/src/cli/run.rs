//! The runner: parse, build the context, call the op, print the result.
//!
//! Order of writes (spec 5.2): the value goes to stdout after `post_run`;
//! the error line is the last write to stderr, after `post_run`, so the
//! app's log text never lands behind it.

use super::io::{Io, SharedWriter};
use super::parse::{argv_redactor, globals_redactor, secret_flags};
use super::{parse, Fallback, Parsed, UsageFailure};
use crate::{
    exit, notice_line, render, write_error, CallOptions, CallPath, CliContext, Notice, NoticeSink,
    Notices, OpKind, ParseCx, Redactor, Registry, Resolve, Style, TraceParent,
};
use coded_error::{CodedError, ErrorClass};
use futures_util::StreamExt;
use std::ffi::OsString;
use std::io::{ErrorKind, Write};
use std::process::ExitCode;
use std::sync::Arc;

/// Callbacks around a run, for apps that start telemetry or record usage.
/// Every method has a no-op default.
pub trait RunHooks {
    /// Before parsing.
    fn pre_parse(&self, argv: &[OsString]) {
        let _ = argv;
    }
    /// A clap or library usage error, before the error line is written.
    fn on_usage_error(&self, failure: &UsageFailure) {
        let _ = failure;
    }
    /// After the op (or its failure), before the runner writes the result or
    /// the error line. Flush logs here.
    fn post_run(&self, outcome: &Outcome) {
        let _ = outcome;
    }
}

/// Hooks that do nothing.
pub struct NoHooks;

impl RunHooks for NoHooks {}

/// How a run ended. Its `Debug` prints the redacted message, never the
/// raw error.
#[non_exhaustive]
pub enum Outcome {
    /// The op succeeded.
    Ok {
        /// The op's name.
        op: String,
        /// The process exit code.
        exit: u8,
    },
    /// The run failed.
    Err {
        /// The op's name, once one was selected.
        op: Option<String>,
        /// What went wrong. It may carry secret values: print `message`
        /// instead.
        error: anyhow::Error,
        /// The error's whole context chain with the run's secrets redacted:
        /// the message of the error line.
        message: String,
    },
}

impl std::fmt::Debug for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Outcome::Ok { op, exit } => f
                .debug_struct("Ok")
                .field("op", op)
                .field("exit", exit)
                .finish(),
            Outcome::Err { op, error, message } => f
                .debug_struct("Err")
                .field("op", op)
                .field("code", &coded_error::facts_anyhow(error).code)
                .field("message", message)
                .finish(),
        }
    }
}

impl Outcome {
    /// The redacted error message of a failed run; `None` on success.
    pub fn message(&self) -> Option<&str> {
        match self {
            Outcome::Ok { .. } => None,
            Outcome::Err { message, .. } => Some(message),
        }
    }

    /// The process exit code of this outcome.
    pub fn exit_code(&self) -> u8 {
        match self {
            Outcome::Ok { exit, .. } => *exit,
            Outcome::Err { error, .. } => exit::exit_code(error),
        }
    }
}

/// A usage failure as a coded error, so the error line and the exit table
/// treat it like any other.
#[derive(Debug)]
struct UsageCoded(UsageFailure);

impl std::fmt::Display for UsageCoded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.message)
    }
}

impl std::error::Error for UsageCoded {}

impl CodedError for UsageCoded {
    fn code(&self) -> &'static str {
        self.0.code
    }
    fn class(&self) -> ErrorClass {
        ErrorClass::Logical
    }
}

/// Runs the command line on the process streams and returns the exit code.
/// It never calls `process::exit`.
pub async fn run<C: CliContext + Resolve>(
    registry: &Registry<C>,
    argv: Vec<OsString>,
    hooks: &dyn RunHooks,
) -> ExitCode {
    ExitCode::from(run_with_io(registry, argv, hooks, Io::std()).await)
}

/// Runs the command line on `io` and returns the exit code.
pub async fn run_with_io<C: CliContext + Resolve>(
    registry: &Registry<C>,
    argv: Vec<OsString>,
    hooks: &dyn RunHooks,
    mut io: Io,
) -> u8 {
    hooks.pre_parse(&argv);
    let scrub = || argv_redactor(&secret_flags(registry), &argv);
    let (parsed, globals) = match parse(registry, &argv) {
        Ok(parsed) => parsed,
        Err(mut failure) => {
            failure.message = scrub().redact(&failure.message);
            hooks.on_usage_error(&failure);
            let error = coded_error::into_anyhow(UsageCoded(failure));
            return finish_err(&mut io, Some(hooks), None, error, &Redactor::default()).exit_code();
        }
    };
    if let Parsed::Help(text) | Parsed::Version(text) = &parsed {
        let _ = write_line(&mut io, text.trim_end());
        return exit::OK;
    }
    // Extras and fallbacks read the globals themselves and need no context.
    if matches!(parsed, Parsed::Extra { .. } | Parsed::Fallback { .. }) {
        let secrets = parsed_secrets::<C>(&parsed);
        return run_unrouted(registry, parsed, &mut io, Some(hooks), &secrets)
            .await
            .exit_code();
    }
    let globals = globals.expect("parse gives globals for everything but help and version");
    let ctx = match C::from_globals(&globals) {
        Ok(ctx) => ctx,
        Err(error) => {
            let mut redactor = parsed_secrets::<C>(&parsed);
            redactor.extend(&scrub());
            return finish_err(&mut io, Some(hooks), None, error, &redactor).exit_code();
        }
    };
    execute(registry, &ctx, parsed, &mut io, Some(hooks))
        .await
        .exit_code()
}

/// Runs one parsed command against a built context and writes its result or
/// its error line. It knows no hooks: use [`run_with_io`] for those.
pub async fn dispatch<C: CliContext + Resolve>(
    registry: &Registry<C>,
    ctx: &C,
    parsed: Parsed,
    io: &mut Io,
) -> Outcome {
    execute(registry, ctx, parsed, io, None).await
}

/// Notices on stderr: JSON lines when piped, text on a terminal.
struct StderrSink {
    out: SharedWriter,
    tty: bool,
}

impl NoticeSink for StderrSink {
    fn send(&self, notice: Notice) {
        let line = if self.tty {
            match &notice {
                Notice::Text(text) => format!("notice: {text}"),
                Notice::Progress {
                    done,
                    total: Some(total),
                    label,
                } => format!("progress: {done}/{total} {label}"),
                Notice::Progress {
                    done,
                    total: None,
                    label,
                } => format!("progress: {done} {label}"),
            }
        } else {
            render(&notice_line(&notice), Style::Compact)
        };
        let _ = writeln!(self.out.clone(), "{line}");
    }
}

fn trace_from_env() -> TraceParent {
    let var = |name: &str| std::env::var(name).ok();
    TraceParent {
        traceparent: var("TRACEPARENT"),
        tracestate: var("TRACESTATE"),
        baggage: var("BAGGAGE"),
    }
}

/// Writes one line to stdout and flushes it.
fn write_line(io: &mut Io, line: &str) -> std::io::Result<()> {
    writeln!(io.stdout, "{line}")?;
    io.stdout.flush()
}

/// Ends a failed run: `post_run` (when there are hooks), then the error line
/// as the last write to stderr.
fn finish_err(
    io: &mut Io,
    hooks: Option<&dyn RunHooks>,
    op: Option<String>,
    error: anyhow::Error,
    redactor: &Redactor,
) -> Outcome {
    let message = redactor.redact(&format!("{error:#}"));
    let outcome = Outcome::Err { op, error, message };
    if let Some(hooks) = hooks {
        hooks.post_run(&outcome);
    }
    if let Outcome::Err { error, .. } = &outcome {
        let _ = write_error(&mut io.stderr, error, redactor, io.stderr_tty);
    }
    outcome
}

/// The result of a stdout write: a closed pipe ends the run quietly.
enum Wrote {
    Done,
    Closed,
    Failed(std::io::Error),
}

fn wrote(result: std::io::Result<()>) -> Wrote {
    match result {
        Ok(()) => Wrote::Done,
        Err(e) if e.kind() == ErrorKind::BrokenPipe => Wrote::Closed,
        Err(e) => Wrote::Failed(e),
    }
}

/// The secret globals' values in the matches a command line selected.
fn parsed_secrets<C: CliContext>(parsed: &Parsed) -> Redactor {
    match parsed {
        Parsed::Op { matches, .. }
        | Parsed::Extra { matches, .. }
        | Parsed::Fallback { matches, .. } => globals_redactor::<C>(matches),
        Parsed::Help(_) | Parsed::Version(_) => Redactor::default(),
    }
}

/// The raw values given to the op's secret arguments.
fn matches_redactor(ids: &[String], matches: &clap::ArgMatches, cx: &ParseCx) -> Redactor {
    let mut redactor = Redactor::default();
    for id in ids {
        if let Ok(Some(values)) = matches.try_get_raw(id) {
            for value in values {
                redactor.add(value.to_string_lossy().as_bytes());
            }
        }
    }
    for secret in &cx.secrets {
        redactor.add(secret);
    }
    redactor
}

/// Runs an extra or the fallback: its exit code is the outcome.
async fn run_unrouted<C>(
    registry: &Registry<C>,
    parsed: Parsed,
    io: &mut Io,
    hooks: Option<&dyn RunHooks>,
    secrets: &Redactor,
) -> Outcome {
    let (op, result) = match parsed {
        Parsed::Extra { name, matches } => {
            let extra = registry
                .extras
                .iter()
                .find(|e| e.command.get_name() == name)
                .expect("parse names a registered extra");
            let result = (extra.run)(&matches).await;
            (name, result)
        }
        Parsed::Fallback { args, matches } => match registry.fallback.as_ref() {
            Some(Fallback::FlagsOnly { name, run, .. }) => (name.to_string(), Ok(run(args).await)),
            Some(Fallback::FreeText { name, run, .. }) => {
                (name.to_string(), run(&matches, args).await)
            }
            None => unreachable!("parse gives a fallback only when the registry has one"),
        },
        _ => unreachable!("only extras and fallbacks are run here"),
    };
    match result {
        Ok(exit) => {
            let outcome = Outcome::Ok { op, exit };
            if let Some(hooks) = hooks {
                hooks.post_run(&outcome);
            }
            outcome
        }
        Err(error) => finish_err(io, hooks, Some(op), error, secrets),
    }
}

async fn execute<C: CliContext + Resolve>(
    registry: &Registry<C>,
    ctx: &C,
    parsed: Parsed,
    io: &mut Io,
    hooks: Option<&dyn RunHooks>,
) -> Outcome {
    let secrets = parsed_secrets::<C>(&parsed);
    let (name, matches, style, text, cursor) = match parsed {
        Parsed::Op {
            name,
            matches,
            style,
            text,
            cursor,
        } => (name, matches, style, text, cursor),
        Parsed::Help(text) | Parsed::Version(text) => {
            let _ = write_line(io, text.trim_end());
            return Outcome::Ok {
                op: String::new(),
                exit: exit::OK,
            };
        }
        parsed @ (Parsed::Extra { .. } | Parsed::Fallback { .. }) => {
            return run_unrouted(registry, parsed, io, hooks, &secrets).await;
        }
    };

    let entry = match registry.entry(&name) {
        Ok(entry) => entry,
        Err(error) => return finish_err(io, hooks, Some(name), error, &secrets),
    };
    let mut cx = match io.stdin.take() {
        Some(stdin) => ParseCx::with_stdin(stdin),
        None => ParseCx::default(),
    };
    let params = match (entry.decode_matches)(&matches, &mut cx) {
        Ok(params) => params,
        Err(error) => {
            // Nothing is built yet, so the redactor comes from the raw values.
            let mut redactor = matches_redactor(&entry.secret_ids, &matches, &cx);
            redactor.extend(&secrets);
            if exit::exit_code(&error) == exit::USAGE {
                if let Some(hooks) = hooks {
                    hooks.on_usage_error(&UsageFailure {
                        code: coded_error::facts_anyhow(&error).code,
                        message: redactor.redact(&format!("{error:#}")),
                    });
                }
            }
            return finish_err(io, hooks, Some(name), error, &redactor);
        }
    };
    drop(cx);

    let mut opts = CallOptions::default()
        .with_trace(trace_from_env())
        .with_notices(Notices::new(Arc::new(StderrSink {
            out: io.stderr.clone(),
            tty: io.stderr_tty,
        })));
    if let Some(caller) = ctx.caller() {
        opts = opts.with_caller(caller);
    }

    if entry.descriptor.kind == OpKind::Stream {
        let invocation = registry
            .call_stream_decoded(&name, params, ctx, opts, cursor, CallPath::Cli, secrets)
            .await;
        let redactor = invocation.redactor;
        let mut stream = match invocation.result {
            Ok(stream) => stream,
            Err(error) => return finish_err(io, hooks, Some(name), error, &redactor),
        };
        while let Some(item) = stream.next().await {
            let line = match item {
                Ok(value) => render(&value, Style::Compact),
                Err(error) => return finish_err(io, hooks, Some(name), error, &redactor),
            };
            match wrote(write_line(io, &line)) {
                Wrote::Done => {}
                Wrote::Closed => break,
                Wrote::Failed(e) => {
                    return finish_err(io, hooks, Some(name), e.into(), &redactor);
                }
            }
        }
        let outcome = Outcome::Ok {
            op: name,
            exit: exit::OK,
        };
        if let Some(hooks) = hooks {
            hooks.post_run(&outcome);
        }
        return outcome;
    }

    let invocation = registry
        .call_decoded(&name, params, ctx, opts, CallPath::Cli, text, secrets)
        .await;
    let redactor = invocation.redactor;
    let value = match invocation.result {
        Ok(value) => value,
        Err(error) => return finish_err(io, hooks, Some(name), error, &redactor),
    };
    let outcome = Outcome::Ok {
        op: name.clone(),
        exit: invocation.exit,
    };
    if let Some(hooks) = hooks {
        hooks.post_run(&outcome);
    }
    let line = match invocation.text {
        Some(text) => text,
        None => render(&value, Style::detect(io.stdout_tty, io.stderr_tty, style)),
    };
    match wrote(write_line(io, &line)) {
        Wrote::Done => outcome,
        Wrote::Closed => Outcome::Ok {
            op: name,
            exit: exit::OK,
        },
        // post_run has run already: only the error line remains.
        Wrote::Failed(e) => finish_err(io, None, Some(name), e.into(), &redactor),
    }
}
