use semver::{BuildMetadata, Prerelease, Version};

use crate::conventional::error::BumpError;
use crate::conventional::version::{Increment, IncrementCommand};
use crate::git::rev::CommitRange;
use crate::{Commit, Tag};

impl Tag {
    pub fn manual_bump(&self, version: &str) -> Result<Self, semver::Error> {
        let mut next = self.clone();
        next.version = Version::parse(version)?;
        Ok(next.reset_metadata())
    }

    pub fn bump_fixed(&self, increment: Increment) -> Self {
        let mut next = self.clone();
        increment.add_to_version(&mut next.version);
        next.reset_metadata()
    }

    pub fn bump(
        &self,
        increment: &IncrementCommand,
        history: &CommitRange,
    ) -> Result<Self, BumpError> {
        let hist_increment = history.get_increment(self.version.major == 0);
        Ok(match increment {
            IncrementCommand::Auto => self.bump_fixed(hist_increment?),
            IncrementCommand::Fixed(increment) => self.bump_fixed(*increment),
            IncrementCommand::AutoMonoRepoGlobal(pkg_increment) => {
                self.auto_global_bump(hist_increment, *pkg_increment)?
            }
            IncrementCommand::Manual(version) => self.manual_bump(version)?,
        })
    }

    fn auto_global_bump(
        &self,
        hist_increment: Result<Increment, BumpError>,
        pkg_increment: Option<Increment>,
    ) -> Result<Self, BumpError> {
        match (pkg_increment, hist_increment) {
            (Some(pkg_increment), Ok(hist_increment)) => {
                Ok(self.bump_fixed(pkg_increment.max(hist_increment)))
            }
            (Some(pkg_increment), Err(_)) => Ok(self.bump_fixed(pkg_increment)),
            (None, Ok(hist_increment)) => Ok(self.bump_fixed(hist_increment)),
            (None, Err(err)) => Err(err),
        }
    }

    pub(crate) fn reset_metadata(mut self) -> Self {
        self.version.build = BuildMetadata::EMPTY;
        self.version.pre = Prerelease::EMPTY;
        self.oid = None;
        self
    }

    pub fn version_increment_from_commit_history(
        &self,
        commits: &[Commit],
    ) -> Result<Increment, BumpError> {
        commits
            .iter()
            .map(|commit| commit.increment(self.version.major == 0))
            .max()
            .ok_or(BumpError::NoCommitFound)
    }
}

impl<'repo> CommitRange<'repo> {
    pub fn get_increment(&self, ignore_breaking: bool) -> Result<Increment, BumpError> {
        self.commits
            .iter()
            .filter_map(|(commit, _)| crate::Commit::from_git_commit(commit).ok())
            .map(|commit| commit.increment(ignore_breaking))
            .max()
            .ok_or(BumpError::NoCommitFound)
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::str::FromStr;

    use anyhow::Result;
    use chrono::Utc;
    use cmd_lib::run_cmd;
    use conventional_commit_parser::commit::{CommitType, ConventionalCommit};
    use sealed_test::prelude::*;
    use semver::Version;
    use speculoos::prelude::*;

    use crate::conventional::commit::{Commit, CommitConfig};
    use crate::conventional::error::BumpError;
    use crate::conventional::version::{Increment, IncrementCommand};
    use crate::git::repository::Repository;
    use crate::git::tag::Tag;
    use crate::settings::{MonoRepoPackage, Settings};
    use crate::test_helpers::{git_init_no_gpg, git_tag};

    impl Commit {
        fn commit_fixture(commit_type: CommitType, is_breaking_change: bool) -> Self {
            Commit {
                oid: "1234".to_string(),
                conventional: ConventionalCommit {
                    commit_type,
                    scope: None,
                    body: None,
                    summary: "message".to_string(),
                    is_breaking_change,
                    footers: vec![],
                },
                author: "".to_string(),
                date: Utc::now().naive_local(),
            }
        }
    }

    #[sealed_test]
    fn major_bump() -> Result<()> {
        // Arrange
        let base_version = Tag::from_str("1.0.0", None)?;

        // Act
        let tag = base_version.bump_fixed(Increment::Major);

        // Assert
        assert_that!(tag.version).is_equal_to(Version::new(2, 0, 0));
        Ok(())
    }

    #[sealed_test]
    fn minor_bump() -> Result<()> {
        // Arrange
        let base_version = Tag::from_str("1.0.0", None)?;

        // Act
        let tag = base_version.bump_fixed(Increment::Minor);

        // Assert
        assert_that!(tag.version).is_equal_to(Version::new(1, 1, 0));
        Ok(())
    }

    #[sealed_test]
    fn patch_bump() -> Result<()> {
        // Arrange
        let base_version = Tag::from_str("1.0.0", None)?;

        // Act
        let tag = base_version.bump_fixed(Increment::Patch);

        // Assert
        assert_that!(tag.version).is_equal_to(Version::new(1, 0, 1));
        Ok(())
    }

    #[sealed_test]
    fn no_bump() -> Result<()> {
        // Arrange
        let base_version = Tag::from_str("1.0.0", None)?;

        // Act
        let tag = base_version.bump_fixed(Increment::NoBump);

        // Assert
        assert_that!(tag.version).is_equal_to(Version::new(1, 0, 0));
        Ok(())
    }

    #[test]
    fn should_get_next_auto_version_patch() -> Result<()> {
        // Arrange
        let patch = Commit::commit_fixture(CommitType::BugFix, false);
        let base_version = Tag::from_str("1.0.0", None)?;

        // Act
        let increment = base_version.version_increment_from_commit_history(&[patch]);

        // Assert
        assert_that!(increment)
            .is_ok()
            .is_equal_to(Increment::Patch);

        Ok(())
    }

    #[test]
    fn should_not_bump_versions_due_to_non_bump_commits() -> Result<()> {
        // Arrange
        let revert = Commit::commit_fixture(CommitType::Revert, false);
        let perf = Commit::commit_fixture(CommitType::Performances, false);
        let documentation = Commit::commit_fixture(CommitType::Documentation, false);
        let chore = Commit::commit_fixture(CommitType::Chore, false);
        let style = Commit::commit_fixture(CommitType::Style, false);
        let refactor = Commit::commit_fixture(CommitType::Refactor, false);
        let test = Commit::commit_fixture(CommitType::Test, false);
        let build = Commit::commit_fixture(CommitType::Build, false);
        let ci = Commit::commit_fixture(CommitType::Ci, false);

        let base_version = Tag::from_str("1.0.0", None)?;

        // Act
        let increment = base_version.version_increment_from_commit_history(&[
            revert,
            perf,
            documentation,
            chore,
            style,
            refactor,
            test,
            build,
            ci,
        ]);

        // Assert
        assert_that!(increment)
            .is_ok()
            .is_equal_to(Increment::NoBump);

        Ok(())
    }

    #[test]
    fn increment_minor_version_should_set_patch_to_zero() -> Result<()> {
        // Arrange
        let version = Tag::from_str("1.1.1", None)?;

        // Act
        let tag = version.bump_fixed(Increment::Minor);

        // Assert
        assert_that!(tag.version).is_equal_to(Version::from_str("1.2.0")?);

        Ok(())
    }

    #[sealed_test]
    fn increment_major_version_should_set_minor_and_patch_to_zero() -> Result<()> {
        // Arrange
        let version = Tag::from_str("1.1.1", None)?;

        // Act
        let tag = version.bump_fixed(Increment::Major);

        // Assert
        assert_that!(tag.version).is_equal_to(Version::from_str("2.0.0")?);

        Ok(())
    }

    #[sealed_test]
    fn increment_should_strip_metadata() -> Result<()> {
        // Arrange
        let version = Tag::from_str("1.1.1-pre+10.1", None)?;

        // Act
        let tag = version.bump_fixed(Increment::Patch);

        // Assert
        assert_that!(tag.version).is_equal_to(Version::from_str("1.1.2")?);

        Ok(())
    }

    #[test]
    fn should_get_next_auto_version_breaking_changes() -> Result<()> {
        // Arrange
        let feature = Commit::commit_fixture(CommitType::Feature, false);
        let breaking_change = Commit::commit_fixture(CommitType::Feature, true);
        let base_version = Tag::from_str("1.0.0", None)?;

        // Act
        let version =
            base_version.version_increment_from_commit_history(&[feature, breaking_change]);

        // Assert
        assert_that!(version).is_ok().is_equal_to(Increment::Major);

        Ok(())
    }

    #[test]
    fn should_get_next_auto_version_breaking_changes_on_initial_dev_version() -> Result<()> {
        // Arrange
        let feature = Commit::commit_fixture(CommitType::Feature, false);
        let breaking_change = Commit::commit_fixture(CommitType::Feature, true);
        let base_version = Tag::from_str("0.1.0", None)?;

        // Act
        let version =
            base_version.version_increment_from_commit_history(&[feature, breaking_change]);

        // Assert
        assert_that!(version).is_ok().is_equal_to(Increment::Minor);

        Ok(())
    }

    #[test]
    fn should_get_next_auto_version_minor() -> Result<()> {
        // Arrange
        let patch = Commit::commit_fixture(CommitType::BugFix, false);
        let feature = Commit::commit_fixture(CommitType::Feature, false);
        let base_version = Tag::from_str("0.1.0", None)?;

        // Act
        let version = base_version.version_increment_from_commit_history(&[patch, feature]);

        // Assert
        assert_that!(version).is_ok().is_equal_to(Increment::Minor);

        Ok(())
    }

    #[sealed_test]
    fn should_get_next_auto_version_minor_with_custom_commit_type() -> Result<()> {
        // Arrange
        git_init_no_gpg()?;
        let mut commit_types = HashMap::new();
        commit_types.insert("ex".to_string(), CommitConfig::new("Ex").with_minor_bump());
        let settings = Settings {
            commit_types,
            ..Default::default()
        };

        let settings = toml::to_string(&settings)?;
        fs::write("cog.toml", settings)?;

        let patch = Commit::commit_fixture(CommitType::BugFix, false);
        let feature = Commit::commit_fixture(CommitType::Custom("ex".to_string()), false);
        let base_version = Tag::from_str("0.1.0", None)?;

        // Act
        let version = base_version.version_increment_from_commit_history(&[patch, feature]);

        // Assert
        assert_that!(version).is_ok().is_equal_to(Increment::Minor);

        Ok(())
    }

    #[sealed_test]
    fn should_get_next_auto_version_patch_with_custom_commit_type() -> Result<()> {
        // Arrange
        git_init_no_gpg()?;
        let mut commit_types = HashMap::new();
        commit_types.insert("ex".to_string(), CommitConfig::new("Ex").with_patch_bump());
        let settings = Settings {
            commit_types,
            ..Default::default()
        };

        let settings = toml::to_string(&settings)?;
        fs::write("cog.toml", settings)?;

        let patch = Commit::commit_fixture(CommitType::Chore, false);
        let feature = Commit::commit_fixture(CommitType::Custom("ex".to_string()), false);
        let base_version = Tag::from_str("0.1.0", None)?;

        // Act
        let version = base_version.version_increment_from_commit_history(&[patch, feature]);

        // Assert
        assert_that!(version).is_ok().is_equal_to(Increment::Patch);

        Ok(())
    }

    #[sealed_test]
    fn should_override_bump_behavior_for_existing_commit_type() -> Result<()> {
        // Arrange
        git_init_no_gpg()?;
        let mut commit_types = HashMap::new();
        commit_types.insert(
            "perf".to_string(),
            CommitConfig::new("Perf").with_minor_bump(),
        );
        let settings = Settings {
            commit_types,
            ..Default::default()
        };

        let settings = toml::to_string(&settings)?;
        fs::write("cog.toml", settings)?;

        let patch = Commit::commit_fixture(CommitType::Chore, false);
        let feature = Commit::commit_fixture(CommitType::Performances, false);
        let base_version = Tag::from_str("0.1.0", None)?;

        // Act
        let version = base_version.version_increment_from_commit_history(&[patch, feature]);

        // Assert
        assert_that!(version).is_ok().is_equal_to(Increment::Minor);

        Ok(())
    }

    #[test]
    fn should_not_fail_without_feature_bug_fix_or_breaking_change_commit() -> Result<()> {
        // Arrange
        let chore = Commit::commit_fixture(CommitType::Chore, false);
        let docs = Commit::commit_fixture(CommitType::Documentation, false);
        let base_version = Tag::from_str("0.1.0", None)?;

        // Act
        let version = base_version.version_increment_from_commit_history(&[chore, docs]);

        // Assert
        assert_that!(version).is_ok().is_equal_to(Increment::NoBump);

        Ok(())
    }

    #[sealed_test]
    fn get_global_monorepo_version_from_history_should_fail_with_only_package_commit() -> Result<()>
    {
        // Arrange
        let repository = init_monorepo()?;
        git_tag("1.0.0")?;
        run_cmd!(
            echo "feature" > one/file;
            git add .;
            git commit -m "feat: feature package one";
        )?;

        let base_version = Tag::from_str("1.0.0", None)?;

        // Act
        let history = repository.revwalk_pattern("1.0.0..")?.global()?;
        assert_that!(history.commits).is_empty();
        let tag = base_version.bump(&IncrementCommand::Auto, &history);

        // Assert
        assert_that!(tag)
            .is_err()
            .matches(|err| matches!(err, BumpError::NoCommitFound));

        Ok(())
    }

    #[sealed_test]
    fn monorepo_auto_bump_should_succeed_with_only_package_commits() -> Result<()> {
        // Act
        let tag = Tag::default()
            .auto_global_bump(Err(BumpError::NoCommitFound), Some(Increment::Minor))?;

        // Assert
        assert_that!(tag.version).is_equal_to(Version::new(0, 1, 0));

        Ok(())
    }

    #[sealed_test]
    fn monorepo_auto_bump_should_succeed_with_only_global_commits() -> Result<()> {
        // Act
        let tag = Tag::default().auto_global_bump(Ok(Increment::Minor), None)?;

        // Assert
        assert_that!(tag.version).is_equal_to(Version::new(0, 1, 0));

        Ok(())
    }

    #[sealed_test]
    fn monorepo_auto_bump_should_succeed_selecting_history_bump() -> Result<()> {
        // Act
        let tag = Tag::default().auto_global_bump(Ok(Increment::Patch), Some(Increment::Minor))?;

        // Assert
        assert_that!(tag.version).is_equal_to(Version::new(0, 1, 0));

        Ok(())
    }

    #[sealed_test]
    fn monorepo_auto_bump_should_succeed_selecting_package_bump() -> Result<()> {
        // Act
        let tag = Tag::default().auto_global_bump(Ok(Increment::Minor), Some(Increment::Patch))?;

        // Assert
        assert_that!(tag.version).is_equal_to(Version::new(0, 1, 0));

        Ok(())
    }

    #[sealed_test]
    fn monorepo_auto_bump_should_succeed_with_equals_history_and_package() -> Result<()> {
        // Act
        let tag = Tag::default().auto_global_bump(Ok(Increment::Minor), Some(Increment::Minor))?;

        // Assert
        assert_that!(tag.version).is_equal_to(Version::new(0, 1, 0));

        Ok(())
    }

    fn init_monorepo() -> Result<Repository> {
        let repository = git_init_no_gpg()?;
        let mut packages = HashMap::new();
        packages.insert(
            "one".to_string(),
            MonoRepoPackage {
                path: PathBuf::from("one"),
                ..Default::default()
            },
        );

        let settings = Settings {
            packages,
            ..Default::default()
        };

        let settings = toml::to_string(&settings)?;

        run_cmd!(
            echo $settings > cog.toml;
            git add .;
            git commit -m "chore: first commit";
            mkdir -p one;
        )?;

        Ok(repository)
    }
}
