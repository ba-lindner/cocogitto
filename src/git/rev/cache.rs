use git2::Oid;

use crate::git::error::Git2Error;
use crate::git::repository::Repository;
use crate::git::tag::Tag;
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Display;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    pub oid: Oid,
    pub tags: Vec<Tag>,
}

impl Display for CommitInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(tag) = self.tags.iter().find(|tag| tag.package.is_none()) {
            tag.fmt(f)
        } else if let Some(tag) = self.tags.first() {
            tag.fmt(f)
        } else {
            self.oid.fmt(f)
        }
    }
}

impl CommitInfo {
    pub fn new(oid: Oid) -> CommitInfo {
        Self {
            oid,
            tags: Vec::new(),
        }
    }

    pub fn from_tag(tag: &Tag) -> Self {
        Self {
            oid: tag.oid.unwrap_or(Oid::zero()),
            tags: vec![tag.clone()],
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

    fn build(repo: &Repository) -> Result<Self, Git2Error> {
        let mut res = Self::empty();

        res.head = repo.get_head_commit()?.id();
        res.insert_oid(res.head);

        res.first = {
            let mut revwalk = repo.0.revwalk()?;
            revwalk.push_head()?;
            revwalk
                .last()
                .expect("No revision found")
                .map_err(Git2Error::CommitNotFound)?
        };
        res.insert_oid(res.first);

        let tags = repo.0.tag_names(None)?;
        let tags = tags.into_iter().flatten();
        for name in tags {
            let Some((oid, tag)) = repo.resolve_tag(name, res.head) else {
                continue;
            };
            res.insert_oid(oid).tags.push(tag.clone());
            res.tags.push(tag);
            res.refs.insert(name.to_string(), oid);
        }

        Ok(res)
    }

    fn insert_oid(&mut self, oid: Oid) -> &mut CommitInfo {
        let long = oid.to_string();
        let short = long[..7].to_string();
        self.refs.insert(long, oid);
        self.refs.insert(short, oid);
        self.commits.entry(oid).or_insert(CommitInfo::new(oid))
    }

    pub fn get_info(&self, oid: Oid) -> Cow<'_, CommitInfo> {
        if let Some(info) = self.commits.get(&oid) {
            Cow::Borrowed(info)
        } else {
            Cow::Owned(CommitInfo::new(oid))
        }
    }
}

impl Repository {
    pub fn get_cache(&self) -> &RepoCache {
        self.1
            .get_or_init(|| RepoCache::build(self).expect("failed to build cache"))
    }

    /// get commit oid from revspec
    ///
    /// this uses the repo cache, so don't try to fill it this way
    pub fn resolve_commit_oid(&self, spec: &str) -> Result<Oid, Git2Error> {
        // first, try cache
        let from_cache = self.get_cache().refs.get(spec).copied();
        // fallback to repository
        let from_repo = || {
            self.0
                .revparse_single(spec)
                .and_then(|obj| obj.peel_to_commit())
                .map(|commit| commit.id())
                .ok()
        };
        from_cache
            .or_else(from_repo)
            .ok_or(Git2Error::UnknownRevision(spec.to_string()))
    }

    fn resolve_tag(&self, tag: &str, head: Oid) -> Option<(Oid, Tag)> {
        let full_name = format!("refs/tags/{tag}");
        let oid = self
            .0
            .find_reference(&full_name)
            .ok()?
            .peel_to_commit()
            .ok()?
            .id();

        // check if tag is reachable from HEAD
        if head != oid && !self.0.graph_descendant_of(head, oid).ok()? {
            return None;
        }

        let res = Tag::from_str(tag, Some(oid)).ok()?;
        Some((oid, res))
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
        assert_that!(cache.refs.is_empty()).is_false();
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
        let head = repo.get_cache().head;

        // Act
        let tag = repo.resolve_tag("1.0.0", head);

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
        let head = repo.get_cache().head;

        // Act
        let tag = repo.resolve_tag("1.0.0", head);

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
        let head = repo.get_cache().head;

        // Act
        let tag = repo.resolve_tag("the_taaaag", head);

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
        let head = repo.get_cache().head;

        // Act
        let tag = repo.resolve_tag("1.0.0", head);

        // Assert
        assert_that!(tag).is_none();
        Ok(())
    }
}
