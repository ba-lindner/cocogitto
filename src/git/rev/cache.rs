use git2::Oid;

use crate::git::error::Git2Error;
use crate::git::oid::OidOf;
use crate::git::repository::Repository;
use crate::git::tag::Tag;
use std::collections::HashMap;
use std::sync::MutexGuard;

#[derive(Debug)]
pub(crate) struct CommitInfo {
    pub is_head: bool,
    pub is_first: bool,
    pub oid: Oid,
    pub tags: Vec<Tag>,
}

impl CommitInfo {
    fn new(oid: Oid) -> CommitInfo {
        Self {
            is_head: false,
            is_first: false,
            oid,
            tags: Vec::new(),
        }
    }

    pub fn as_oid_of(&self, package: Option<&str>) -> OidOf {
        let tag = self
            .tags
            .iter()
            .filter(|tag| tag.package.as_deref() == package)
            .next();
        match tag {
            Some(tag) => OidOf::Tag(tag.clone()),
            None if self.is_first => OidOf::FirstCommit(self.oid),
            None if self.is_head => OidOf::Head(self.oid),
            None => OidOf::Other(self.oid),
        }
    }
}

#[derive(Debug)]
pub(crate) struct RepoCache {
    pub head: Oid,
    pub first: Oid,
    pub refs: HashMap<String, Oid>,
    pub commits: HashMap<Oid, CommitInfo>,
    pub tags: Vec<Tag>,
}

impl RepoCache {
    pub fn empty() -> Self {
        Self {
            head: Oid::zero(),
            first: Oid::zero(),
            refs: HashMap::new(),
            commits: HashMap::new(),
            tags: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.head = Oid::zero();
        self.first = Oid::zero();
        self.refs.clear();
        self.commits.clear();
        self.tags.clear();
    }

    fn is_empty(&self) -> bool {
        self.refs.is_empty() && self.commits.is_empty()
    }

    fn fill(&mut self, repo: &Repository) -> Result<(), Git2Error> {
        self.head = repo.get_head_commit_oid()?;
        self.insert_oid(self.head).is_head = true;

        self.first = repo.get_first_commit()?;
        self.insert_oid(self.first).is_first = true;

        let tags = repo.0.tag_names(None)?;
        let tags = tags
            .into_iter()
            .flatten()
            .filter_map(|tag| repo.resolve_tag(tag));
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
}

impl Repository {
    pub fn get_cache(&self) -> MutexGuard<'_, RepoCache> {
        let mut cache = self.1.lock().unwrap();
        if cache.is_empty() {
            cache.fill(self).expect("failed to construct tag cache");
        }
        cache
    }

    fn resolve_tag<'t>(&'_ self, tag: &'t str) -> Option<(&'t str, Oid, Tag)> {
        let full_name = format!("refs/tags/{tag}");
        let oid = self
            .0
            .find_reference(&full_name)
            .ok()?
            .peel_to_commit()
            .ok()?
            .id();

        // check if tag is reachable from HEAD
        let head = self.get_head_commit_oid().ok()?;
        if head != oid && !self.0.graph_descendant_of(head, oid).ok()? {
            return None;
        }

        let res = Tag::from_str(tag, Some(oid)).ok()?;
        Some((tag, oid, res))
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
        assert_that!(tag).is_some();
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
        assert_that!(tag).is_some();
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
        assert_that!(tag).is_none();
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
        assert_that!(tag).is_none();
        Ok(())
    }
}
