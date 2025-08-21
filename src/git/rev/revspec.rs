use std::fmt::Display;

use crate::git::error::Git2Error;
use crate::git::repository::Repository;
use crate::git::tag::Tag;
use git2::Oid;

#[derive(Debug, Default, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct RevSpec {
    pub from: Option<Oid>,
    pub to: Option<Oid>,
}

impl RevSpec {
    pub fn from_tag(tag: &Tag) -> Self {
        Self {
            from: tag.oid,
            to: None,
        }
    }

    pub fn parse(spec: &str, repository: &Repository) -> Result<Self, Git2Error> {
        if let Some((from, to)) = spec.split_once("..") {
            let parse_part = |part: &str| {
                (!part.is_empty())
                    .then(|| repository.resolve_commit_oid(part))
                    .transpose()
            };
            Ok(Self {
                from: parse_part(from)?,
                to: parse_part(to)?,
            })
        } else if let Ok(tag) = Tag::from_str(spec, None) {
            let prev = repository.get_previous_tag(&tag).and_then(|prev| prev.oid);
            let curr = repository.resolve_commit_oid(spec)?;
            Ok(Self {
                from: prev,
                to: Some(curr),
            })
        } else {
            Err(Git2Error::InvalidCommitRangePattern(spec.to_string()))
        }
    }
}

impl Display for RevSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(from) = self.from {
            from.fmt(f)?;
        }
        f.write_str("..")?;
        if let Some(to) = self.to {
            to.fmt(f)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::git::rev::revspec::RevSpec;
    use crate::test_helpers::{commit, git_init_no_gpg, git_tag};
    use anyhow::Result;
    use git2::Oid;
    use sealed_test::prelude::*;
    use speculoos::prelude::*;

    // these tests need to go somewhere else

    // #[sealed_test]
    // fn should_resolve_tag_oid() -> Result<()> {
    //     let repo = git_init_no_gpg()?;
    //     commit("chore: first commit")?;
    //     let commit_oid = commit("chore: v1")?;
    //     git_tag("1.0.0")?;

    //     let oid = repo.resolve_oid_of("1.0.0")?;

    //     assert_that!(oid).is_equal_to(OidOf::Tag(Tag {
    //         package: None,
    //         prefix: None,
    //         version: Version::new(1, 0, 0),
    //         oid: Some(Oid::from_str(&commit_oid)?),
    //     }));

    //     Ok(())
    // }

    // #[sealed_test]
    // fn should_resolve_head_oid() -> Result<()> {
    //     let repo = git_init_no_gpg()?;
    //     commit("chore: first commit")?;
    //     let commit_oid = commit("chore: other")?;

    //     let oid = repo.resolve_oid_of(&commit_oid)?;

    //     assert_that!(oid).is_equal_to(OidOf::Head(Oid::from_str(&commit_oid)?));

    //     Ok(())
    // }

    // #[sealed_test]
    // fn should_resolve_commit_oid() -> Result<()> {
    //     let repo = git_init_no_gpg()?;
    //     commit("chore: first commit")?;
    //     let commit_oid = commit("chore: other")?;
    //     commit("chore: head")?;

    //     let oid = repo.resolve_oid_of(&commit_oid)?;

    //     assert_that!(oid).is_equal_to(OidOf::Other(Oid::from_str(&commit_oid)?));

    //     Ok(())
    // }

    // #[sealed_test]
    // fn should_error_on_unknown_rev() -> Result<()> {
    //     let repo = git_init_no_gpg()?;
    //     commit("chore: first commit")?;
    //     commit("chore: other")?;

    //     let oid = repo.resolve_oid_of("v32");

    //     assert_that!(oid).is_err();

    //     Ok(())
    // }

    #[sealed_test]
    fn convert_str_to_pattern_to() -> Result<()> {
        let repo = git_init_no_gpg()?;
        commit("chore: first commit")?;
        let oid = commit("chore: v1")?;
        git_tag("1.0.0")?;

        let pattern = RevSpec::parse("..1.0.0", &repo);

        assert_that!(pattern).is_ok().is_equal_to(RevSpec {
            from: None,
            to: Some(Oid::from_str(&oid)?),
        });
        Ok(())
    }

    #[sealed_test]
    fn convert_str_to_pattern_from() -> Result<()> {
        let repo = git_init_no_gpg()?;
        commit("chore: first commit")?;
        let oid = commit("chore: v1")?;
        git_tag("1.0.0")?;
        commit("chore: more commits")?;

        let pattern = RevSpec::parse("1.0.0..", &repo);

        assert_that!(pattern).is_ok().is_equal_to(RevSpec {
            from: Some(Oid::from_str(&oid)?),
            to: None,
        });
        Ok(())
    }

    #[sealed_test]
    fn convert_empty_pattern() -> Result<()> {
        let repo = git_init_no_gpg()?;
        commit("chore: first commit")?;

        let pattern = RevSpec::parse("..", &repo);

        assert_that!(pattern).is_ok().is_equal_to(RevSpec {
            from: None,
            to: None,
        });
        Ok(())
    }

    #[sealed_test]
    fn convert_full_pattern() -> Result<()> {
        let repo = git_init_no_gpg()?;
        commit("chore: first commit")?;
        commit("chore: first commit")?;
        let oid_1 = commit("chore: v1")?;
        git_tag("1.0.0")?;
        let oid_2 = commit("chore: more commits")?;
        git_tag("2.0.0")?;

        let pattern = RevSpec::parse("1.0.0..2.0.0", &repo);

        assert_that!(pattern).is_ok().is_equal_to(RevSpec {
            from: Some(Oid::from_str(&oid_1)?),
            to: Some(Oid::from_str(&oid_2)?),
        });
        Ok(())
    }

    #[sealed_test]
    fn revspec_at_tag_without_previous() -> Result<()> {
        let repo = git_init_no_gpg()?;
        commit("chore: first commit")?;
        let oid = commit("chore: v1")?;
        git_tag("1.0.0")?;

        let pattern = RevSpec::parse("1.0.0", &repo);

        assert_that!(pattern).is_ok().is_equal_to(RevSpec {
            from: None,
            to: Some(Oid::from_str(&oid)?),
        });
        Ok(())
    }

    #[sealed_test]
    fn revspec_at_tag_with_previous() -> Result<()> {
        let repo = git_init_no_gpg()?;
        commit("chore: first commit")?;
        commit("chore: first commit")?;
        let oid_1 = commit("chore: v1")?;
        git_tag("1.0.0")?;
        let oid_2 = commit("chore: more commits")?;
        git_tag("2.0.0")?;

        let pattern = RevSpec::parse("2.0.0", &repo);

        assert_that!(pattern).is_ok().is_equal_to(RevSpec {
            from: Some(Oid::from_str(&oid_1)?),
            to: Some(Oid::from_str(&oid_2)?),
        });
        Ok(())
    }
}
