use git2::Commit;
use itertools::Itertools;

use crate::git::oid::OidOf;

pub mod cache;
pub mod filters;
pub mod revspec;
pub mod revwalk;

#[derive(Debug)]
pub struct CommitIter<'repo>(pub(crate) Vec<(OidOf, Commit<'repo>)>);

impl CommitIter<'_> {
    pub fn from_oid(&self) -> OidOf {
        self.0.last().expect("non empty commit range").0.clone()
    }

    pub fn to_oid(&self) -> OidOf {
        self.0.first().expect("non empty commit range").0.clone()
    }

    pub fn iter_commits(&self) -> impl Iterator<Item = &Commit> {
        self.0.iter().map(|(_, commit)| commit)
    }

    pub fn try_retain<E>(
        &mut self,
        f: impl Fn(&OidOf, &Commit<'_>) -> Result<bool, E>,
    ) -> Result<(), E> {
        let filter: Vec<_> = self
            .0
            .iter()
            .map(|(oid, commit)| f(oid, commit))
            .try_collect()?;
        assert_eq!(filter.len(), self.0.len());
        let mut filter = filter.into_iter();
        self.0.retain(|_| filter.next().unwrap());
        Ok(())
    }
}

impl<'repo> IntoIterator for CommitIter<'repo> {
    type Item = (OidOf, Commit<'repo>);
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
