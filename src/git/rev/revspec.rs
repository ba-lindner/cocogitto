use crate::git::error::Git2Error;
use crate::git::oid::OidOf;
use crate::git::repository::Repository;
use crate::git::rev::cache::get_cache;
use crate::git::tag::Tag;
use git2::Oid;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RevSpecPattern2 {
    pub from: Option<Oid>,
    pub to: Option<Oid>,
}

impl Repository {
    pub(super) fn revspec_from_str(&self, s: &str) -> Result<RevSpecPattern2, Git2Error> {
        if let Some((from, to)) = s.split_once("..") {
            let from = if from.is_empty() {
                None
            } else {
                Some(*self.resolve_oid_of(from)?.oid())
            };

            let to = if to.is_empty() {
                None
            } else {
                Some(*self.resolve_oid_of(to)?.oid())
            };

            Ok(RevSpecPattern2 { from, to })
        } else if let Ok(tag) = Tag::from_str(s, None) {
            let previous = self.get_previous_tag(&tag).and_then(|tag| tag.oid);

            Ok(RevSpecPattern2 {
                from: previous,
                to: Some(*self.resolve_oid_of(s)?.oid()),
            })
        } else {
            Err(Git2Error::InvalidCommitRangePattern(s.to_string()))
        }
    }

    pub(super) fn resolve_oid_of(&self, from: &str) -> Result<OidOf, Git2Error> {
        self.resolve_oid_of_package(from, None)
    }

    pub(super) fn resolve_oid_of_package(
        &self,
        from: &str,
        package: Option<&str>,
    ) -> Result<OidOf, Git2Error> {
        let cache = get_cache(self);

        // note: `get` cannot be used as `starts_with` is used for comparison
        let oids = cache
            .iter()
            .find(|(k, _)| k.starts_with(from))
            .map(|(_, v)| v);

        let oid = oids
            .and_then(|v| {
                v.iter()
                    .filter(|oid| {
                        if let OidOf::Tag(tag) = oid {
                            tag.package.as_deref() == package
                        } else {
                            true
                        }
                    })
                    .next_back()
            })
            .cloned();
        if let Some(oid) = oid {
            Ok(oid)
        } else {
            let object = self
                .0
                .revparse_single(from)
                .map_err(|_| Git2Error::UnknownRevision(from.to_string()))?;
            Ok(OidOf::Other(object.id()))
        }
    }
}

#[cfg(test)]
mod test {
    use crate::git::oid::OidOf;
    use crate::git::rev::revspec::RevSpecPattern2;
    use crate::git::tag::Tag;
    use crate::test_helpers::{commit, git_init_no_gpg, git_tag};
    use anyhow::Result;
    use git2::Oid;
    use sealed_test::prelude::*;
    use semver::Version;
    use speculoos::prelude::*;

    #[sealed_test]
    fn should_resolve_tag_oid() -> Result<()> {
        let repo = git_init_no_gpg()?;
        commit("chore: first commit")?;
        let commit_oid = commit("chore: v1")?;
        git_tag("1.0.0")?;

        let oid = repo.resolve_oid_of("1.0.0")?;

        assert_that!(oid).is_equal_to(OidOf::Tag(Tag {
            package: None,
            prefix: None,
            version: Version::new(1, 0, 0),
            oid: Some(Oid::from_str(&commit_oid)?),
        }));

        Ok(())
    }

    #[sealed_test]
    fn should_resolve_head_oid() -> Result<()> {
        let repo = git_init_no_gpg()?;
        commit("chore: first commit")?;
        let commit_oid = commit("chore: other")?;

        let oid = repo.resolve_oid_of(&commit_oid)?;

        assert_that!(oid).is_equal_to(OidOf::Head(Oid::from_str(&commit_oid)?));

        Ok(())
    }

    #[sealed_test]
    fn should_resolve_commit_oid() -> Result<()> {
        let repo = git_init_no_gpg()?;
        commit("chore: first commit")?;
        let commit_oid = commit("chore: other")?;
        commit("chore: head")?;

        let oid = repo.resolve_oid_of(&commit_oid)?;

        assert_that!(oid).is_equal_to(OidOf::Other(Oid::from_str(&commit_oid)?));

        Ok(())
    }

    #[sealed_test]
    fn should_error_on_unknown_rev() -> Result<()> {
        let repo = git_init_no_gpg()?;
        commit("chore: first commit")?;
        commit("chore: other")?;

        let oid = repo.resolve_oid_of("v32");

        assert_that!(oid).is_err();

        Ok(())
    }

    #[sealed_test]
    fn convert_str_to_pattern_to() -> Result<()> {
        let repo = git_init_no_gpg()?;
        commit("chore: first commit")?;
        let sha_1 = commit("chore: v1")?;
        git_tag("1.0.0")?;

        let pattern = repo.revspec_from_str("..1.0.0")?;

        assert_that!(pattern).is_equal_to(RevSpecPattern2 {
            from: None,
            to: Some(sha_1.parse().unwrap()),
        });
        Ok(())
    }

    #[sealed_test]
    fn convert_str_to_pattern_from() -> Result<()> {
        let repo = git_init_no_gpg()?;
        commit("chore: first commit")?;
        let sha_1 = commit("chore: v1")?;
        git_tag("1.0.0")?;
        commit("chore: more commits")?;

        let pattern = repo.revspec_from_str("1.0.0..")?;

        assert_that!(pattern).is_equal_to(RevSpecPattern2 {
            from: Some(sha_1.parse().unwrap()),
            to: None,
        });
        Ok(())
    }

    #[sealed_test]
    fn convert_empty_pattern() -> Result<()> {
        let repo = git_init_no_gpg()?;
        commit("chore: first commit")?;

        let pattern = repo.revspec_from_str("..")?;

        assert_that!(pattern).is_equal_to(RevSpecPattern2 {
            from: None,
            to: None,
        });
        Ok(())
    }

    #[sealed_test]
    fn error_invalid_pattern() -> Result<()> {
        let repo = git_init_no_gpg()?;
        commit("chore: first commit")?;
        let sha_1 = commit("chore: v1")?;
        git_tag("1.0.0")?;
        let pattern = repo.revspec_from_str("1.0.0")?;

        assert_that!(pattern).is_equal_to(RevSpecPattern2 {
            from: None,
            to: Some(sha_1.parse().unwrap()),
        });
        Ok(())
    }

    #[sealed_test]
    fn convert_full_pattern() -> Result<()> {
        let repo = git_init_no_gpg()?;
        commit("chore: first commit")?;
        commit("chore: first commit")?;
        let sha_1 = commit("chore: v1")?;
        git_tag("1.0.0")?;
        let sha_2 = commit("chore: more commits")?;
        git_tag("2.0.0")?;

        let pattern = repo.revspec_from_str("1.0.0..2.0.0")?;

        assert_that!(pattern).is_equal_to(RevSpecPattern2 {
            from: Some(sha_1.parse().unwrap()),
            to: Some(sha_2.parse().unwrap()),
        });
        Ok(())
    }
}
