//! Extra (non-op) commands: a clap command the app runs itself, with the
//! registry's globals.

use crate::cli::tree::{check_extra_flags, top_words};
use crate::{CliContext, OpGlobals, Registry};
use clap::{ArgMatches, Command};
use futures_util::future::BoxFuture;
use std::sync::Arc;

type ExtraRun =
    Arc<dyn for<'a> Fn(&'a ArgMatches) -> BoxFuture<'a, anyhow::Result<u8>> + Send + Sync>;

/// An extra command with its erased runner. The runner decodes the globals
/// from the matches itself, so the registry never names `C::Globals`.
pub(crate) struct Extra {
    pub command: Command,
    pub run: ExtraRun,
}

impl<C: CliContext + Sync> Registry<C> {
    /// Adds an extra command at the root. It gets the registry's globals (with
    /// their environment fallback) and runs `run` with the parsed globals and
    /// its own matches. `run` returns the process exit code; an `Err` ends the
    /// run like a failed op: `post_run`, the redacted error line, and the exit
    /// code of the error's class.
    ///
    /// # Panics
    ///
    /// Panics when the command's name is an op or group word, or another
    /// extra's name, or when the command or one of its subcommands has a long
    /// flag that a registry global or the library already owns.
    pub fn push_extra(
        &mut self,
        command: Command,
        run: for<'a> fn(&'a C::Globals, &'a ArgMatches) -> BoxFuture<'a, anyhow::Result<u8>>,
    ) {
        let name = command.get_name().to_owned();
        assert!(
            !top_words(self).contains(&name),
            "extra `{name}` clashes with op word"
        );
        check_extra_flags::<C>(&command);
        self.extras.push(Extra {
            command,
            run: Arc::new(
                move |matches: &ArgMatches| -> BoxFuture<'_, anyhow::Result<u8>> {
                    Box::pin(async move {
                        let globals = <C::Globals as OpGlobals>::from_matches(matches)?;
                        run(&globals, matches).await
                    })
                },
            ),
        });
    }
}
