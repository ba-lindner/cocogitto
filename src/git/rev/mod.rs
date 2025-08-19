use git2::Commit;

use crate::git::{oid::OidOf, rev::cache::CommitInfo};

pub mod cache;
pub mod filters;
pub mod revspec;
pub mod revwalk;

#[derive(Debug)]
pub struct CommitIterOld<'repo>(Vec<(OidOf, Commit<'repo>)>);

impl CommitIterOld<'_> {
    pub fn from_oid(&self) -> OidOf {
        self.0.last().expect("non empty commit range").0.clone()
    }

    pub fn to_oid(&self) -> OidOf {
        self.0.first().expect("non empty commit range").0.clone()
    }

    pub fn iter_commits(&self) -> impl Iterator<Item = &'_ Commit<'_>> {
        self.0.iter().map(|(_, commit)| commit)
    }
}

impl<'repo> IntoIterator for CommitIterOld<'repo> {
    type Item = (OidOf, Commit<'repo>);
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CommitIter<'repo>(pub Vec<(Commit<'repo>, CommitInfo)>);

impl<'repo> IntoIterator for CommitIter<'repo> {
    type Item = (Commit<'repo>, CommitInfo);
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'repo> CommitIter<'repo> {
    pub fn into_old(self, package: Option<&'_ str>) -> CommitIterOld<'repo> {
        CommitIterOld(
            self.into_iter()
                .map(|(commit, info)| (info.as_oid_of(package), commit))
                .collect(),
        )
    }
}
