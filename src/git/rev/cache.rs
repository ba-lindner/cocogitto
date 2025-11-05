use git2::Oid;

use crate::git::error::{Git2Error, TagError};
use crate::git::oid::CommitInfo;
use crate::git::repository::Repository;
use crate::git::tag::Tag;
use std::collections::HashMap;
use std::sync::MutexGuard;

/// Stores commit information, mainly tags
///
/// To avoid having duplicate [`CommitInfo`] instances for the same commit
/// due to mutliple possible references (full/short oid, multiple tag names),
/// the cache is split into two:
/// * `refs` contains all ways to reference a commit and maps those to the commit oid
/// * `commits` contains the actual [`CommitInfo`]
///
/// Additionally, the [`Oid`] of the head commit and first commit
/// aswell as a list of all [`Tag`]s is stored in the cache.
#[derive(Debug)]
pub(crate) struct RepoCache {
    pub refs: HashMap<String, Oid>,
    pub commits: HashMap<Oid, CommitInfo>,
    pub head: Oid,
    pub first: Oid,
    pub tags: Vec<Tag>,
}

impl RepoCache {
    pub fn empty() -> Self {
        Self {
            refs: HashMap::new(),
            commits: HashMap::new(),
            head: Oid::zero(),
            first: Oid::zero(),
            tags: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.refs.clear();
        self.commits.clear();
        self.head = Oid::zero();
        self.first = Oid::zero();
        self.tags.clear();
    }

    fn is_empty(&self) -> bool {
        self.refs.is_empty() && self.commits.is_empty()
    }

    fn fill(&mut self, repo: &Repository) -> Result<(), Git2Error> {
        self.head = repo.get_head_commit_oid()?;
        self.insert_oid(self.head).head = true;

        self.first = repo.get_first_commit()?;
        self.insert_oid(self.first).first = true;

        let tags = repo.0.tag_names(None)?;
        let tags = tags.into_iter().flatten().filter_map(|tag_name| {
            let tag = repo.resolve_tag(tag_name).ok()?;
            Some((tag_name, tag.oid.unwrap(), tag))
        });
        for (name, oid, tag) in tags {
            self.insert_oid(oid).tags.push(tag.clone());
            self.tags.push(tag);
            self.refs.insert(name.to_string(), oid);
        }

        Ok(())
    }

    fn insert_oid(&mut self, oid: Oid) -> &mut CommitInfo {
        let long = oid.to_string();
        let short = long[..7].to_string();
        self.refs.insert(long, oid);
        self.refs.insert(short, oid);
        self.commits.entry(oid).or_insert(CommitInfo::new(oid))
    }

    pub fn get_info(&self, oid: Oid) -> CommitInfo {
        self.commits
            .get(&oid)
            .cloned()
            .unwrap_or(CommitInfo::new(oid))
    }
}

impl Repository {
    pub fn get_cache(&self) -> MutexGuard<'_, RepoCache> {
        let mut cache = self.1.lock().unwrap();
        if cache.is_empty() {
            cache.fill(self).expect("failed to construct tag cache");
        }
        cache
    }

    fn resolve_tag(&self, tag: &str) -> Result<Tag, Git2Error> {
        let oid = self
            .0
            .resolve_reference_from_short_name(tag)
            .map_err(|err| TagError::not_found(tag, err))?
            .peel_to_commit()
            .map_err(|err| TagError::no_commit(tag, err))?
            .id();

        // check if tag is reachable from HEAD
        let head = self.get_head_commit_oid()?;
        if head != oid && !self.0.graph_descendant_of(head, oid)? {
            return Err(Git2Error::TagError(TagError::NotReachableFromHead {
                tag: tag.to_string(),
            }));
        }

        Tag::from_str(tag, Some(oid)).map_err(Git2Error::TagError)
    }
}

#[cfg(test)]
mod test {
    use crate::git::repository::Repository;
    use crate::test_helpers::git_init_no_gpg;
    use cmd_lib::run_cmd;
    use sealed_test::prelude::*;
    use speculoos::prelude::*;

    #[test]
    fn init_cache_ok() -> anyhow::Result<()> {
        let repo = Repository::open(".")?;
        let cache = repo.get_cache();
        assert_that!(cache.is_empty()).is_false();
        Ok(())
    }

    #[sealed_test]
    fn resolve_annotated_tag_ok() -> anyhow::Result<()> {
        // Arrange
        let repo = git_init_no_gpg()?;
        run_cmd!(
            git commit --allow-empty -m "first commit";
            git tag -a 1.0.0 -m "annotated tag";
        )?;

        // Act
        let tag = repo.resolve_tag("1.0.0");

        // Assert
        assert_that!(tag).is_ok();
        Ok(())
    }

    #[sealed_test]
    fn resolve_lightweight_tag_ok() -> anyhow::Result<()> {
        // Arrange
        let repo = git_init_no_gpg()?;
        run_cmd!(
            git commit --allow-empty -m "first commit";
            git tag 1.0.0;
        )?;

        // Act
        let tag = repo.resolve_tag("1.0.0");

        // Assert
        assert_that!(tag).is_ok();
        Ok(())
    }

    #[sealed_test]
    fn resolve_lightweight_tag_err() -> anyhow::Result<()> {
        // Arrange
        let repo = git_init_no_gpg()?;
        run_cmd!(
            git commit --allow-empty -m "first commit";
            git tag the_tag;
        )?;

        // Act
        let tag = repo.resolve_tag("the_taaaag");

        // Assert
        assert_that!(tag).is_err();
        Ok(())
    }

    #[sealed_test]
    fn dont_read_future_tag() -> anyhow::Result<()> {
        // Arrange
        let repo = git_init_no_gpg()?;
        run_cmd!(
            git commit --allow-empty -m "first commit";
            git commit --allow-empty -m "second commit";
            git tag 1.0.0;
            git reset HEAD^
        )?;

        // Act
        let tag = repo.resolve_tag("1.0.0");

        // Assert
        assert_that!(tag).is_err();
        Ok(())
    }
}
