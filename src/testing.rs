//! Testing utilities.
//!
//! This is inside `src` rather than `tests` since we use this code in some unit
//! tests.

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::ops::Deref;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::git::{GitRunInfo, GitVersion, Repo};
use crate::util::get_sh;
use anyhow::Context;
use fn_error_context::context;
use tempfile::TempDir;

const DUMMY_NAME: &str = "Testy McTestface";
const DUMMY_EMAIL: &str = "test@example.com";
const DUMMY_DATE: &str = "Wed 29 Oct 12:34:56 2020 PDT";

/// Wrapper around the Git executable, for testing.
#[derive(Clone, Debug)]
pub struct Git {
    /// The path to the repository on disk. The directory itself must exist,
    /// although it might not have a `.git` folder in it. (Use `Git::init_repo`
    /// to initialize it.)
    pub repo_path: PathBuf,

    /// The path to the Git executable on disk. This is important since we test
    /// against multiple Git versions.
    pub path_to_git: PathBuf,
}

/// Options for `Git::init_repo_with_options`.
#[derive(Debug)]
pub struct GitInitOptions {
    /// If `true`, then `init_repo_with_options` makes an initial commit with
    /// some content.
    pub make_initial_commit: bool,

    /// If `true`, run `git branchless init` as part of initialization process.
    pub run_branchless_init: bool,
}

impl Default for GitInitOptions {
    fn default() -> Self {
        GitInitOptions {
            make_initial_commit: true,
            run_branchless_init: true,
        }
    }
}

/// Options for `Git::run_with_options`.
#[derive(Debug)]
pub struct GitRunOptions {
    /// The timestamp of the command. Mostly useful for `git commit`. This should
    /// be a number like 0, 1, 2, 3...
    pub time: isize,

    /// The exit code that `Git` should return.
    pub expected_exit_code: i32,

    /// The input to write to the child process's stdin.
    pub input: Option<String>,
}

impl Default for GitRunOptions {
    fn default() -> Self {
        GitRunOptions {
            time: 0,
            expected_exit_code: 0,
            input: None,
        }
    }
}

impl Git {
    /// Constructor.
    pub fn new(repo_path: PathBuf, git_run_info: GitRunInfo) -> Self {
        let GitRunInfo {
            path_to_git,
            // We pass the repo directory when calling `run`.
            working_directory: _,
            // We manually set the environment when calling `run`.
            env: _,
        } = git_run_info;
        Git {
            repo_path,
            path_to_git,
        }
    }

    /// Replace dynamic strings in the output, for testing purposes.
    pub fn preprocess_output(&self, stdout: String) -> anyhow::Result<String> {
        let path_to_git = self
            .path_to_git
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Could not convert path to Git to string"))?;
        let stdout = stdout.replace(path_to_git, "<git-executable>");

        // NB: tests which run on Windows are unlikely to succeed due to this
        // `canonicalize` call.
        let repo_path = std::fs::canonicalize(&self.repo_path)?;

        let repo_path = repo_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Could not convert repo path to string"))?;
        let stdout = stdout.replace(&repo_path, "<repo-path>");
        Ok(stdout)
    }

    /// Get the `PATH` environment variable to use for testing.
    pub fn get_path_for_env(&self) -> OsString {
        let cargo_bin_path = assert_cmd::cargo::cargo_bin("git-branchless");
        let branchless_path = cargo_bin_path
            .parent()
            .expect("Unable to find git-branchless path parent");
        let git_path = self
            .path_to_git
            .parent()
            .expect("Unable to find git path parent");
        let bash = get_sh().expect("bash missing?");
        let bash_path = bash.parent().unwrap();
        std::env::join_paths(
            vec![
                // For Git to be able to launch `git-branchless`.
                branchless_path,
                // For our hooks to be able to call back into `git`.
                git_path,
                // For branchless to manually invoke bash when needed.
                bash_path,
            ]
            .iter()
            .map(|path| path.to_str().expect("Unable to decode path component")),
        )
        .expect("joining paths")
    }

    /// Run a Git command.
    #[context("Running Git command with args: {:?} and options: {:?}", args, options)]
    pub fn run_with_options<S: AsRef<str> + std::fmt::Debug>(
        &self,
        args: &[S],
        options: &GitRunOptions,
    ) -> anyhow::Result<(String, String)> {
        let GitRunOptions {
            time,
            expected_exit_code,
            input,
        } = options;

        // Required for determinism, as these values will be baked into the commit
        // hash.
        let date: OsString = format!("{date} -{time:0>2}", date = DUMMY_DATE, time = time).into();

        let args: Vec<&str> = {
            let repo_path = self.repo_path.to_str().expect("Could not decode repo path");
            let mut new_args: Vec<&str> = vec!["-C", repo_path];
            new_args.extend(args.iter().map(|arg| arg.as_ref()));
            new_args
        };

        // Fake "editor" which accepts the default contents of any commit
        // messages. Usually, we can set this with `git commit -m`, but we have
        // no such option for things such as `git rebase`, which may call `git
        // commit` later as a part of their execution.
        //
        // ":" is understood by `git` to skip editing.
        let git_editor = OsString::from(":");

        let new_path = self.get_path_for_env();
        let env: Vec<(&str, &OsStr)> = vec![
            ("GIT_AUTHOR_DATE", &date),
            ("GIT_COMMITTER_DATE", &date),
            ("GIT_EDITOR", &git_editor),
            ("PATH_TO_GIT", &self.path_to_git.as_os_str()),
            ("PATH", &new_path),
        ];

        let mut command = Command::new(&self.path_to_git);
        command.args(&args).env_clear().envs(env.iter().copied());

        let result = if let Some(input) = input {
            let mut child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            write!(child.stdin.take().unwrap(), "{}", &input)?;
            child.wait_with_output().with_context(|| {
                format!(
                    "Running git
                    Executable: {:?}
                    Args: {:?}
                    Stdin: {:?}
                    Env: <not shown>",
                    &self.path_to_git, &args, input
                )
            })?
        } else {
            command.output().with_context(|| {
                format!(
                    "Running git
                    Executable: {:?}
                    Args: {:?}
                    Env: <not shown>",
                    &self.path_to_git, &args
                )
            })?
        };

        let exit_code = result
            .status
            .code()
            .expect("Failed to read exit code from Git process");
        let result = if exit_code != *expected_exit_code {
            anyhow::bail!(
                "Git command {:?} {:?} exited with unexpected code {} (expected {})
                stdout:
                {}
                stderr:
                {}",
                &self.path_to_git,
                &args,
                exit_code,
                expected_exit_code,
                &String::from_utf8_lossy(&result.stdout),
                &String::from_utf8_lossy(&result.stderr),
            )
        } else {
            result
        };
        let stdout = String::from_utf8(result.stdout)?;
        let stdout = self.preprocess_output(stdout)?;
        let stderr = String::from_utf8(result.stderr)?;
        let stderr = self.preprocess_output(stderr)?;
        Ok((stdout, stderr))
    }

    /// Run a Git command.
    pub fn run<S: AsRef<str> + std::fmt::Debug>(
        &self,
        args: &[S],
    ) -> anyhow::Result<(String, String)> {
        self.run_with_options(args, &Default::default())
    }

    /// Set up a Git repo in the directory and initialize git-branchless to work
    /// with it.
    #[context("Initializing Git repo with options: {:?}", options)]
    pub fn init_repo_with_options(&self, options: &GitInitOptions) -> anyhow::Result<()> {
        self.run(&["init"])?;
        self.run(&["config", "user.name", DUMMY_NAME])?;
        self.run(&["config", "user.email", DUMMY_EMAIL])?;

        if options.make_initial_commit {
            self.commit_file("initial", 0)?;
        }

        // Non-deterministic metadata (depends on current time).
        self.run(&["config", "branchless.commitMetadata.relativeTime", "false"])?;

        if options.run_branchless_init {
            self.run(&["branchless", "init"])?;
        }

        Ok(())
    }

    /// Set up a Git repo in the directory and initialize git-branchless to work
    /// with it.
    pub fn init_repo(&self) -> anyhow::Result<()> {
        self.init_repo_with_options(&Default::default())
    }

    /// Write the provided contents to the provided file in the repository root.
    pub fn write_file(&self, name: &str, contents: &str) -> anyhow::Result<()> {
        let file_path = self.repo_path.join(format!("{}.txt", name));
        std::fs::write(&file_path, contents)?;
        Ok(())
    }

    /// Commit a file with default contents. The `time` argument is used to set
    /// the commit timestamp, which is factored into the commit hash.
    #[context(
        "Committing file {:?} at time {:?} with contents: {:?}",
        name,
        time,
        contents
    )]
    pub fn commit_file_with_contents(
        &self,
        name: &str,
        time: isize,
        contents: &str,
    ) -> anyhow::Result<git2::Oid> {
        self.write_file(name, contents)?;
        self.run(&["add", "."])?;
        self.run_with_options(
            &["commit", "-m", &format!("create {}.txt", name)],
            &GitRunOptions {
                time,
                ..Default::default()
            },
        )?;

        let repo = self.get_repo()?;
        let oid = repo
            .get_head_info()?
            .oid
            .expect("Could not find OID for just-created commit");
        Ok(oid)
    }

    /// Commit a file with default contents. The `time` argument is used to set
    /// the commit timestamp, which is factored into the commit hash.
    pub fn commit_file(&self, name: &str, time: isize) -> anyhow::Result<git2::Oid> {
        self.commit_file_with_contents(name, time, &format!("{} contents\n", name))
    }

    /// Detach HEAD. This is useful to call to make sure that no branch is
    /// checked out, and therefore that future commit operations don't move any
    /// branches.
    #[context("Detaching HEAD")]
    pub fn detach_head(&self) -> anyhow::Result<()> {
        self.run(&["checkout", "--detach"])?;
        Ok(())
    }

    /// Get a `Repo` object for this repository.
    #[context("Getting the `Repo` object for {:?}", self)]
    pub fn get_repo(&self) -> anyhow::Result<Repo> {
        Repo::from_dir(&self.repo_path)
    }

    /// Get the version of the Git executable.
    #[context("Getting the Git version for {:?}", self)]
    pub fn get_version(&self) -> anyhow::Result<GitVersion> {
        let (version_str, _stderr) = self.run(&["version"])?;
        version_str.parse()
    }

    /// Determine if the Git executable supports the `reference-transaction`
    /// hook.
    #[context("Detecting reference-transaction support for {:?}", self)]
    pub fn supports_reference_transactions(&self) -> anyhow::Result<bool> {
        let version = self.get_version()?;
        Ok(version >= GitVersion(2, 29, 0))
    }

    /// Determine if the `--committer-date-is-author-date` option to `git rebase
    /// -i` is respected.
    ///
    /// This affects whether we can rely on the timestamps being preserved
    /// during a rebase when `branchless.restack.preserveTimestamps` is set.
    pub fn supports_committer_date_is_author_date(&self) -> anyhow::Result<bool> {
        // The `--committer-date-is-author-date` option was previously passed
        // only to the `am` rebase back-end, until Git v2.29, when it became
        // available for merge back-end rebases as well.
        //
        // See https://git-scm.com/docs/git-rebase/2.28.0
        //
        // > These flags are passed to git am to easily change the dates of the
        // > rebased commits (see git-am[1]).
        // >
        // > See also INCOMPATIBLE OPTIONS below.
        //
        // See https://git-scm.com/docs/git-rebase/2.29.0
        //
        // > Instead of using the current time as the committer date, use the
        // > author date of the commit being rebased as the committer date. This
        // > option implies --force-rebase.
        let version = self.get_version()?;
        Ok(version >= GitVersion(2, 29, 0))
    }

    /// Resolve a file during a merge or rebase conflict with the provided
    /// contents.
    #[context("Resolving file {:?} with contents: {:?}", name, contents)]
    pub fn resolve_file(&self, name: &str, contents: &str) -> anyhow::Result<()> {
        let file_path = self.repo_path.join(format!("{}.txt", name));
        std::fs::write(&file_path, contents)?;
        let file_path = match file_path.to_str() {
            None => anyhow::bail!("Could not convert file path to string: {:?}", file_path),
            Some(file_path) => file_path,
        };
        self.run(&["add", file_path])?;
        Ok(())
    }
}

/// Get the path to the Git executable for testing.
#[context("Getting the Git executable to use")]
pub fn get_path_to_git() -> anyhow::Result<PathBuf> {
    let path_to_git = std::env::var_os("PATH_TO_GIT").with_context(|| {
        "No path to git set. Try running as: PATH_TO_GIT=$(which git) cargo test ..."
    })?;
    let path_to_git = PathBuf::from(&path_to_git);
    Ok(path_to_git)
}

/// Wrapper around a `Git` instance which cleans up the repository once dropped.
pub struct GitWrapper {
    _repo_dir: TempDir,
    git: Git,
}

impl Deref for GitWrapper {
    type Target = Git;

    fn deref(&self) -> &Self::Target {
        &self.git
    }
}

/// Create a temporary directory for testing and a `Git` instance to use with it.
pub fn make_git() -> anyhow::Result<GitWrapper> {
    let repo_dir = tempfile::tempdir()?;
    let path_to_git = get_path_to_git()?;
    let path_to_git = GitRunInfo {
        path_to_git,
        working_directory: repo_dir.path().to_path_buf(),
        env: std::env::vars_os().collect(),
    };
    let git = Git::new(repo_dir.path().to_path_buf(), path_to_git);
    Ok(GitWrapper {
        _repo_dir: repo_dir,
        git,
    })
}
