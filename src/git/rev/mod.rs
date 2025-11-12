use git2::Commit;
use itertools::Itertools as _;

use crate::git::error::Git2Error;
use crate::git::oid::{CommitInfo, ReleaseVersion};
use crate::git::repository::Repository;
use crate::git::rev::revspec::RevSpecPattern2;
use crate::Target;

pub mod cache;
pub mod filters;
pub mod revspec;
pub mod revwalk;

#[derive(Debug)]
pub struct CommitIter<'repo> {
    commits: Vec<(CommitInfo, Commit<'repo>)>,
    spec: RevSpecPattern2,
    repository: &'repo Repository,
    pub(crate) target: Target,
}

impl<'repo> CommitIter<'repo> {
    pub fn from_oid(&self) -> CommitInfo {
        self.commits
            .last()
            .expect("non empty commit range")
            .0
            .clone()
    }

    pub fn to_oid(&self) -> CommitInfo {
        self.commits
            .first()
            .expect("non empty commit range")
            .0
            .clone()
    }

    pub fn iter_commits(&self) -> impl Iterator<Item = &'_ Commit<'_>> {
        self.commits.iter().map(|(_, commit)| commit)
    }

    pub fn is_empty(&self) -> bool {
        self.commits.is_empty()
    }

    pub fn version_range(&self) -> (ReleaseVersion, ReleaseVersion) {
        let cache = self.repository.get_cache();
        let from = cache
            .get_info(self.spec.from.unwrap_or(cache.first))
            .into_release_version(self.target);
        let to = cache
            .get_info(self.spec.to.unwrap_or(cache.head))
            .into_release_version(self.target);
        (from, to)
    }

    pub(crate) fn set_target(mut self, target: Target) -> Result<Self, Git2Error> {
        if self.target == target {
            return Ok(self);
        }
        self.target = target;
        let to_retain = self
            .iter_commits()
            .map(|commit| target.matches_commit(self.repository, commit))
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(to_retain.len(), self.commits.len());
        let mut to_retain = to_retain.into_iter();
        self.commits
            .retain(|_| to_retain.next().expect("asserted that enough items exist"));
        Ok(self)
    }

    pub fn split_at_tags(&self) -> Result<Vec<CommitIter<'repo>>, Git2Error> {
        let mut oids = self
            .commits
            .iter()
            .flat_map(|(info, _)| &info.tags)
            .filter(|tag| self.target.matches_tag(tag))
            .sorted()
            .map(|tag| tag.oid)
            .collect::<Vec<_>>();
        if oids.last() != Some(&self.spec.to) {
            oids.push(self.spec.to);
        }
        let mut last_from = self.spec.from;
        let mut res = Vec::new();
        for oid in oids {
            res.push(
                self.repository
                    .revwalk(RevSpecPattern2 {
                        from: last_from,
                        to: oid,
                    })?
                    .set_target(self.target)?,
            );
            last_from = oid;
        }
        res.retain(|iter| !iter.is_empty());
        Ok(res)
    }
}

impl<'repo> IntoIterator for CommitIter<'repo> {
    type Item = (CommitInfo, Commit<'repo>);
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.commits.into_iter()
    }
}
