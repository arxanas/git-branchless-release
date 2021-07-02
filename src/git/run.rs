use std::collections::HashMap;
use std::convert::TryInto;
use std::ffi::OsString;
use std::io::{stderr, stdout, Write};
use std::path::PathBuf;
use std::process::Command;

use anyhow::Context;
use fn_error_context::context;

use crate::core::eventlog::{EventTransactionId, BRANCHLESS_TRANSACTION_ID_ENV_VAR};

/// Path to the `git` executable on disk to be executed.
#[derive(Clone, Debug)]
pub struct GitRunInfo {
    /// The path to the Git executable on disk.
    pub path_to_git: PathBuf,

    /// The working directory that the Git executable should be run in.
    pub working_directory: PathBuf,

    /// The environment variables that should be passed to the Git process.
    pub env: HashMap<OsString, OsString>,
}

impl GitRunInfo {
    /// Run Git in a subprocess, and inform the user.
    ///
    /// This is suitable for commands which affect the working copy or should run
    /// hooks. We don't want our process to be responsible for that.
    ///
    /// `args` contains the list of arguments to pass to Git, not including the Git
    /// executable itself.
    ///
    /// Returns the exit code of Git (non-zero signifies error).
    #[context("Running Git ({:?}) with args: {:?}", &self, args)]
    #[must_use = "The return code for `run_git` must be checked"]
    pub fn run<S: AsRef<str> + std::fmt::Debug>(
        &self,
        event_tx_id: Option<EventTransactionId>,
        args: &[S],
    ) -> anyhow::Result<isize> {
        let GitRunInfo {
            path_to_git,
            working_directory,
            env,
        } = self;
        println!(
            "branchless: {} {}",
            path_to_git.to_string_lossy(),
            args.iter()
                .map(|arg| arg.as_ref())
                .collect::<Vec<_>>()
                .join(" ")
        );
        stdout().flush()?;
        stderr().flush()?;

        let mut command = Command::new(path_to_git);
        command.current_dir(working_directory);
        command.args(args.iter().map(|arg| arg.as_ref()));
        command.env_clear();
        command.envs(env.iter());
        if let Some(event_tx_id) = event_tx_id {
            command.env(BRANCHLESS_TRANSACTION_ID_ENV_VAR, event_tx_id.to_string());
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("Spawning Git subrocess: {:?} {:?}", path_to_git, args))?;
        let exit_status = child.wait().with_context(|| {
            format!(
                "Waiting for Git subprocess to complete: {:?} {:?}",
                path_to_git, args
            )
        })?;

        // On Unix, if the child process was terminated by a signal, we need to call
        // some Unix-specific functions to access the signal that terminated it. For
        // simplicity, just return `1` in those cases.
        let exit_code = exit_status.code().unwrap_or(1);
        let exit_code = exit_code
            .try_into()
            .with_context(|| format!("Converting exit code {} from i32 to isize", exit_code))?;
        Ok(exit_code)
    }
}