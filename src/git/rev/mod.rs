use git2::{Commit, Oid};

use crate::git::{
    repository::Repository,
    rev::{cache::CommitInfo, filters::CommitRangeFilter},
};

pub mod cache;
pub mod filters;
pub mod revspec;
pub mod revwalk;

#[derive(Debug, Clone)]
pub struct CommitRange<'repo> {
    pub commits: Vec<Commit<'repo>>,
    repository: &'repo Repository,
}

impl<'repo> IntoIterator for CommitRange<'repo> {
    type Item = Commit<'repo>;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.commits.into_iter()
    }
}

impl<'a, 'repo> IntoIterator for &'a CommitRange<'repo> {
    type Item = &'a Commit<'repo>;
    type IntoIter = std::slice::Iter<'a, Commit<'repo>>;

    fn into_iter(self) -> Self::IntoIter {
        self.commits.iter()
    }
}

impl<'repo> CommitRange<'repo> {
    pub fn iter(&self) -> <&Self as IntoIterator>::IntoIter {
        self.into_iter()
    }

    pub fn from(&self) -> CommitInfo {
        self.commits
            .last()
            .map(|commit| {
                self.repository
                    .get_cache()
                    .get_info(commit.id())
                    .into_owned()
            })
            .unwrap_or(CommitInfo::new(Oid::zero()))
    }

    pub fn to(&self) -> CommitInfo {
        self.commits
            .first()
            .map(|commit| {
                self.repository
                    .get_cache()
                    .get_info(commit.id())
                    .into_owned()
            })
            .unwrap_or(CommitInfo::new(Oid::zero()))
    }

    pub fn split_at_tags(&self, filter: CommitRangeFilter<'_>) -> Vec<CommitRange<'repo>> {
        let mut ranges = Vec::new();
        let mut current = Vec::new();
        for commit in self {
            let info = self.repository.get_cache().get_info(commit.id());
            if !current.is_empty() && info.tags.iter().any(|tag| filter.matches_tag(tag)) {
                ranges.push(CommitRange {
                    commits: current,
                    repository: self.repository,
                });
                current = vec![commit.clone()];
            } else {
                current.push(commit.clone());
            }
        }
        if !current.is_empty() {
            ranges.push(CommitRange {
                commits: current,
                repository: self.repository,
            });
        }
        ranges
    }
}
