use git2::{Commit, Oid};

use crate::git::{repository::Repository, rev::cache::CommitInfo};

pub mod cache;
pub mod filters;
pub mod revspec;
pub mod revwalk;

#[derive(Debug, Clone)]
pub struct CommitRange<'repo> {
    pub commits: Vec<(Commit<'repo>, CommitInfo)>,
    repository: &'repo Repository,
}

impl<'repo> IntoIterator for CommitRange<'repo> {
    type Item = (Commit<'repo>, CommitInfo);
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.commits.into_iter()
    }
}

impl<'repo> CommitRange<'repo> {
    pub fn from(&self) -> CommitInfo {
        self.commits
            .last()
            .map(|(_, info)| info.clone())
            .unwrap_or(CommitInfo::new(Oid::zero()))
    }
}
