use semver::Version;

use crate::conventional::error::BumpError;
use crate::conventional::version::Increment;
use crate::git::rev::revspec::RevSpecPattern2;
use crate::target::Target;
use crate::{Commit, IncrementCommand, Repository, Tag};

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum IncrementStrategy {
    Fixed(Increment),
    Auto(Target, Option<Increment>),
    Manual(String),
}

impl IncrementCommand {
    pub(crate) fn as_strategy(
        &self,
        target: Target,
        package_increment: Option<Increment>,
    ) -> IncrementStrategy {
        match self {
            IncrementCommand::Auto => IncrementStrategy::Auto(target, package_increment),
            IncrementCommand::Major => IncrementStrategy::Fixed(Increment::Major),
            IncrementCommand::Minor => IncrementStrategy::Fixed(Increment::Minor),
            IncrementCommand::Patch => IncrementStrategy::Fixed(Increment::Patch),
            IncrementCommand::Manual(version) => IncrementStrategy::Manual(version.clone()),
        }
    }
}

impl Tag {
    fn fixed_bump(&self, increment: Increment) -> Version {
        match increment {
            Increment::Major => Version::new(self.version.major + 1, 0, 0),
            Increment::Minor => Version::new(self.version.major, self.version.minor + 1, 0),
            Increment::Patch => Version::new(
                self.version.major,
                self.version.minor,
                self.version.patch + 1,
            ),
            Increment::NoBump => {
                Version::new(self.version.major, self.version.minor, self.version.patch)
            }
        }
    }

    fn auto_bump(
        &self,
        repository: &Repository,
        target: Target,
        package_increment: Option<Increment>,
    ) -> Result<Version, BumpError> {
        let version_from_history = self.get_version_from_commit_history(repository, target);
        match (package_increment, version_from_history) {
            (Some(package_increment), Ok(version_from_history)) => {
                let tag_from_packages = self.fixed_bump(package_increment);
                Ok(tag_from_packages.max(version_from_history))
            }
            (Some(package_increment), Err(_)) => {
                let tag_from_packages = self.fixed_bump(package_increment);
                Ok(tag_from_packages)
            }
            (None, Ok(tag_from_history)) => Ok(tag_from_history),
            (None, Err(err)) => Err(err),
        }
    }

    pub(crate) fn bump(
        &self,
        increment: IncrementStrategy,
        repository: &Repository,
    ) -> Result<Version, BumpError> {
        match increment {
            IncrementStrategy::Fixed(increment) => Ok(self.fixed_bump(increment)),
            IncrementStrategy::Auto(target, package_increment) => {
                self.auto_bump(repository, target, package_increment)
            }
            IncrementStrategy::Manual(version) => Version::parse(&version).map_err(Into::into),
        }
    }

    fn get_version_from_commit_history(
        &self,
        repository: &Repository,
        target: Target,
    ) -> Result<Version, BumpError> {
        let changelog_start_oid = repository
            .get_latest_tag(self.package.as_deref(), false)
            .ok()
            .and_then(|tag| tag.oid);

        let conventional_commits: Vec<Commit> = repository
            .revwalk(RevSpecPattern2 {
                from: changelog_start_oid,
                to: None,
            })?
            .for_target(target)?
            .ignore_commits_by_settings()
            .iter_commits()
            .map(|commit| Commit::from_git_commit(commit))
            .filter_map(Result::ok)
            .collect();

        let increment = self.version_increment_from_commit_history(&conventional_commits)?;

        Ok(self.fixed_bump(increment))
    }

    pub fn version_increment_from_commit_history(
        &self,
        commits: &[Commit],
    ) -> Result<Increment, BumpError> {
        commits
            .iter()
            .map(|commit| commit.increment(self.version.major > 0))
            .max()
            .ok_or(BumpError::NoCommitFound)
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;

    use anyhow::Result;
    use chrono::Utc;
    use cmd_lib::run_cmd;
    use conventional_commit_parser::commit::{CommitType, ConventionalCommit};
    use sealed_test::prelude::*;
    use semver::Version;
    use speculoos::prelude::*;

    use crate::conventional::commit::{Commit, CommitConfig};
    use crate::conventional::error::BumpError;
    use crate::conventional::version::Increment;
    use crate::git::repository::Repository;
    use crate::git::tag::Tag;
    use crate::settings::{MonoRepoPackage, MonorepoConfig, Settings};
    use crate::target::Target;
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

    fn fixed_bump_test(base: &str, increment: Increment, expected: Version) -> Result<()> {
        // Arrange
        let base_version = Tag::from_str(base, None)?;

        // Act
        let version = base_version.fixed_bump(increment);

        // Assert
        assert_that!(version).is_equal_to(expected);
        Ok(())
    }

    #[test]
    fn major_bump() -> Result<()> {
        fixed_bump_test("1.0.0", Increment::Major, Version::new(2, 0, 0))
    }

    #[test]
    fn minor_bump() -> Result<()> {
        fixed_bump_test("1.0.0", Increment::Minor, Version::new(1, 1, 0))
    }

    #[test]
    fn patch_bump() -> Result<()> {
        fixed_bump_test("1.0.0", Increment::Patch, Version::new(1, 0, 1))
    }

    #[test]
    fn no_bump() -> Result<()> {
        fixed_bump_test("1.0.0", Increment::NoBump, Version::new(1, 0, 0))
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
        fixed_bump_test("1.1.1", Increment::Minor, Version::new(1, 2, 0))
    }

    #[test]
    fn increment_major_version_should_set_minor_and_patch_to_zero() -> Result<()> {
        fixed_bump_test("1.1.1", Increment::Major, Version::new(2, 0, 0))
    }

    #[test]
    fn increment_should_strip_metadata() -> Result<()> {
        fixed_bump_test("1.1.1-pre+10.0", Increment::Patch, Version::new(1, 1, 2))
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
        run_cmd!(
            echo "feature" > one;
            git add .;
            git commit -m "feat: feature package one";
        )?;
        git_tag("1.0.0")?;

        let base_version = Tag::from_str("1.0.0", None)?;

        // Act
        let tag = base_version
            .get_version_from_commit_history(&repository, Target::Monorepo { manual: false });

        // Assert
        assert_that!(tag)
            .is_err()
            .matches(|err| matches!(err, BumpError::NoCommitFound));

        Ok(())
    }

    #[sealed_test]
    fn monorepo_auto_bump_should_succeed_with_only_package_commits() -> Result<()> {
        // Arrange
        let repository = git_init_no_gpg()?;
        run_cmd!(
            echo "feature" > one;
            git add .;
            git commit -m "feat: feature package one";
        )?;
        let base_version = Tag::from_str("0.1.0", None)?;

        // Act
        let version = base_version.auto_bump(
            &repository,
            Target::Monorepo { manual: false },
            Some(Increment::Minor),
        )?;

        // Assert
        assert_that!(version).is_equal_to(Version::new(0, 2, 0));

        Ok(())
    }

    macro_rules! monorepo_test {
        // uses , instead of . to avoid parsing version as float
        ($pkg:expr => $major:literal , $minor:literal , $patch:literal ; $($setup:tt)*) => {
            // Arrange
            let repository = init_monorepo()?;

            run_cmd!(
                $($setup)*
            )?;

            // Act
            let version = Tag::default().auto_bump(&repository, Target::Monorepo { manual: false }, $pkg)?;

            // Assert
            assert_that!(version).is_equal_to(Version::new($major, $minor, $patch));

            Ok(())
        }
    }

    #[sealed_test]
    fn monorepo_auto_bump_should_succeed_with_only_global_commits() -> Result<()> {
        monorepo_test! {
            None => 0,1,0;
            echo "global" > global;
            git add .;
            git commit -m "feat: non package commit";
        }
    }

    #[sealed_test]
    fn monorepo_auto_bump_should_succeed_selecting_history_bump() -> Result<()> {
        monorepo_test! {
            Some(Increment::Minor) => 0,1,0;
            // Patch increment from global commits
            // Minor increment from package bumps
            echo "global" > global;
            git add .;
            git commit -m "fix: global fix";
            echo "feature" > one;
            git add .;
            git commit -m "feat: feature 1 package one";
        }
    }

    #[sealed_test]
    fn monorepo_auto_bump_should_succeed_selecting_package_bump() -> Result<()> {
        monorepo_test! {
            Some(Increment::Patch) => 0,1,0;
            // Minor increment from global commits
            // Patch increment from package bumps
            echo "global" > global;
            git add .;
            git commit -m "feat: global fix";
            echo "feature" > one;
            git add .;
            git commit -m "fix: fix 1 package one";
        }
    }

    #[sealed_test]
    fn monorepo_auto_bump_should_succeed_with_equals_history_and_package() -> Result<()> {
        monorepo_test! {
            Some(Increment::Minor) => 0,1,0;
            // Minor increment from global commits
            // Minor increment from package bumps
            echo "global" > global;
            git add .;
            git commit -m "feat: global fix";
            echo "feature" > one;
            git add .;
            git commit -m "feature: package one";
        }
    }

    #[sealed_test]
    fn monorepo_auto_bump_should_succeed_with_mixed_commit() -> Result<()> {
        // Arrange
        let repository = init_monorepo()?;

        // Minor increment from global commits
        // Minor increment from package bumps
        run_cmd!(
            echo "start" > start;
            git add .;
            git commit -m "chore: version";
            git tag "0.1.0";
            git tag "one-0.1.0";
            echo "feature" > one;
            echo "global" > global;
            git add .;
            git commit -m "feature: package one and global";
        )?;

        // Act
        let version = Tag::from_str("0.1.0", None)?.auto_bump(
            &repository,
            Target::Monorepo { manual: false },
            Some(Increment::Minor),
        )?;

        // Assert
        assert_that!(version).is_equal_to(Version::new(0, 2, 0));

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
            monorepo: Some(MonorepoConfig {
                packages,
                ..Default::default()
            }),
            ..Default::default()
        };

        let settings = toml::to_string(&settings)?;

        run_cmd!(
            echo $settings > cog.toml;
            git add .;
            git commit -m "chore: first commit";
        )?;

        Ok(repository)
    }
}
