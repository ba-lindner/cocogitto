use git2::Commit;

use crate::git::{
    error::Git2Error, oid::CommitInfo, repository::Repository, rev::filters::CommitFilter,
};

pub mod cache;
pub mod filters;
pub mod revspec;
pub mod revwalk;

#[derive(Debug)]
pub struct CommitIter<'repo>(Vec<(CommitInfo, Commit<'repo>)>);

impl<'repo> CommitIter<'repo> {
    pub fn single(oid_of: CommitInfo, commit: Commit<'repo>) -> Self {
        CommitIter::<'repo>(vec![(oid_of, commit)])
    }

    pub fn from_oid(&self) -> CommitInfo {
        self.0.last().expect("non empty commit range").0.clone()
    }

    pub fn to_oid(&self) -> CommitInfo {
        self.0.first().expect("non empty commit range").0.clone()
    }

    pub fn iter_commits(&self) -> impl Iterator<Item = &'_ Commit<'_>> {
        self.0.iter().map(|(_, commit)| commit)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn apply_filter(
        mut self,
        repository: &Repository,
        filter: CommitFilter,
    ) -> Result<Self, Git2Error> {
        let to_retain = self
            .0
            .iter()
            .map(|(_, commit)| filter.matches_commit(repository, commit))
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(to_retain.len(), self.0.len());
        let mut to_retain = to_retain.into_iter();
        self.0
            .retain(|_| to_retain.next().expect("asserted that enough items exist"));
        Ok(self)
    }
}

impl<'repo> IntoIterator for CommitIter<'repo> {
    type Item = (CommitInfo, Commit<'repo>);
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
