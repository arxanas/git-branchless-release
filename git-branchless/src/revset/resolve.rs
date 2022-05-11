use itertools::Itertools;
use lib::core::dag::{CommitSet, Dag};
use lib::core::effects::Effects;
use lib::git::{Commit, Repo};
use tracing::instrument;

/// The result of attempting to resolve commits.
pub enum ResolveCommitsResult<'repo> {
    /// All commits were successfully resolved.
    Ok {
        /// The resolved commit sets, in the order that they were passed in.
        commits: Vec<Commit<'repo>>,
    },

    /// The first commit which couldn't be resolved.
    CommitNotFound {
        /// The identifier of the commit, as provided by the user.
        commit: String,
    },
}

/// Parse strings which refer to commits, such as:
///
/// - Full OIDs.
/// - Short OIDs.
/// - Reference names.
#[instrument]
pub fn resolve_commits<'repo>(
    effects: &Effects,
    repo: &'repo Repo,
    dag: &mut Dag,
    hashes: Vec<String>,
) -> eyre::Result<ResolveCommitsResult<'repo>> {
    let mut commits = Vec::new();
    for hash in hashes {
        let commit = match repo.revparse_single_commit(&hash)? {
            Some(commit) => commit,
            None => return Ok(ResolveCommitsResult::CommitNotFound { commit: hash }),
        };
        commits.push(commit)
    }

    let commit_oids = commits.iter().map(|commit| commit.get_oid()).collect_vec();
    dag.sync_from_oids(
        effects,
        repo,
        CommitSet::empty(),
        commit_oids.into_iter().collect::<CommitSet>(),
    )?;
    Ok(ResolveCommitsResult::Ok { commits })
}