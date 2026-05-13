use git2::{Commit, Oid};

use crate::git::error::Git2Error;
use crate::git::oid::{CommitInfo, ReleaseVersion};
use crate::git::repository::Repository;
use crate::git::rev::revspec::RevSpecPattern2;
use crate::target::Target;

pub mod cache;
pub mod filters;
pub mod revspec;
pub mod revwalk;

#[derive(Debug)]
pub struct CommitIter<'repo> {
    commits: Vec<(CommitInfo, Commit<'repo>)>,
    spec: RevSpecPattern2,
    repository: &'repo Repository,
    target: Target,
}

impl<'repo> CommitIter<'repo> {
    pub fn from_oid(&self) -> CommitInfo {
        let cache = self.repository.get_cache();
        cache.get_info(self.spec.from.unwrap_or(cache.first))
    }

    pub fn to_oid(&self) -> CommitInfo {
        let cache = self.repository.get_cache();
        cache.get_info(self.spec.to.unwrap_or(cache.head))
    }

    pub fn version_range(&self) -> (ReleaseVersion, ReleaseVersion) {
        (
            self.from_oid().into_version(self.target.as_package()),
            self.to_oid().into_version(self.target.as_package()),
        )
    }

    pub fn iter_commits(&self) -> impl Iterator<Item = &'_ Commit<'_>> {
        self.commits.iter().map(|(_, commit)| commit)
    }

    pub fn is_empty(&self) -> bool {
        self.commits.is_empty()
    }

    pub fn for_target(mut self, target: Target) -> Result<Self, Git2Error> {
        self.target = target;
        let mut new_commits = Vec::new();
        for commit in self.commits {
            let include = match target {
                Target::Standard | Target::Unified { .. } => true,
                Target::Package { name, .. } => {
                    self.repository.is_commit_in_package(&commit.1, name)?
                }
                Target::Monorepo { .. } => self.repository.is_commit_global(&commit.1)?,
            };
            if include {
                new_commits.push(commit);
            }
        }
        self.commits = new_commits;
        Ok(self)
    }

    pub fn split_at_tags(self) -> Vec<Self> {
        let mut releases = vec![];
        let mut commit_iter = self.commits.into_iter().rev().peekable();
        let mut from = self.spec.from;

        while commit_iter.peek().is_some() {
            let mut release_commits = vec![];

            let mut to = Option::<Oid>::None;

            for (info, commit) in commit_iter.by_ref() {
                if has_commit_target_version(&info, self.target) {
                    to = Some(info.oid);
                    release_commits.push((info, commit));
                    break;
                }
                release_commits.push((info, commit));
            }

            release_commits.reverse();
            releases.push(CommitIter {
                commits: release_commits,
                spec: RevSpecPattern2 { from, to },
                repository: self.repository,
                target: self.target,
            });
            from = to;
        }

        releases
    }
}

fn has_commit_target_version(info: &CommitInfo, target: Target) -> bool {
    match target {
        Target::Standard | Target::Unified { .. } => !info.tags.is_empty(),
        Target::Package { name, .. } => info
            .tags
            .iter()
            .any(|tag| tag.package.as_deref() == Some(name)),
        Target::Monorepo { .. } => info.tags.iter().any(|tag| tag.package.is_none()),
    }
}

impl<'repo> IntoIterator for CommitIter<'repo> {
    type Item = (CommitInfo, Commit<'repo>);
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.commits.into_iter()
    }
}
