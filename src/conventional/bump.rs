use git2::Commit as Git2Commit;
use semver::{BuildMetadata, Prerelease, Version};

use crate::conventional::error::BumpError;
use crate::conventional::version::Increment;
use crate::git::error::Git2Error;
use crate::git::oid::OidOf;
use crate::git::rev::filters::PackagePathFilter;
use crate::{Commit, IncrementCommand, Repository, Tag, SETTINGS};

pub(crate) fn filter_merge_commits(commit: &Git2Commit) -> bool {
    !SETTINGS.ignore_merge_commits || commit.parent_count() <= 1
}

pub(crate) fn filter_fixup_commits(commit: &Git2Commit) -> bool {
    !SETTINGS.ignore_fixup_commits
        // manual `Option::is_none_or`
        || commit.message().is_none()
        || commit.message().is_some_and(|msg| {
            !msg.starts_with("fixup!") && !msg.starts_with("squash!") && !msg.starts_with("amend!")
        })
}

impl Tag {
    fn manual_bump(&self, version: &str) -> Result<Self, semver::Error> {
        let mut next = self.clone();
        next.version = Version::parse(version)?;
        Ok(next)
    }

    fn major_bump(&self) -> Self {
        let mut next = self.clone();
        next.version.major += 1;
        next.version.minor = 0;
        next.version.patch = 0;
        next.reset_metadata()
    }

    fn minor_bump(&self) -> Self {
        let mut next = self.clone();
        next.version.minor += 1;
        next.version.patch = 0;
        next.reset_metadata()
    }

    fn patch_bump(&self) -> Self {
        let mut next = self.clone();
        next.version.patch += 1;
        next.reset_metadata()
    }

    fn no_bump(&self) -> Self {
        let next = self.clone();
        next.reset_metadata()
    }

    fn auto_global_bump(
        &self,
        repository: &Repository,
        package_increment: Option<Increment>,
    ) -> Result<Self, BumpError>
    where
        Self: Sized,
    {
        let tag_from_history =
            self.get_version_from_history(repository, None, filter_commits_global(repository));
        match (package_increment, tag_from_history) {
            (Some(package_increment), Ok(tag_from_history)) => {
                let tag_from_packages = self.bump(package_increment.into(), repository)?;
                Ok(tag_from_packages.max(tag_from_history))
            }
            (Some(package_increment), Err(_)) => {
                let tag_from_packages = self.bump(package_increment.into(), repository)?;
                Ok(tag_from_packages)
            }
            (None, Ok(tag_from_history)) => Ok(tag_from_history),
            (None, Err(err)) => Err(err),
        }
    }

    pub(crate) fn bump(
        &self,
        increment: IncrementCommand,
        repository: &Repository,
    ) -> Result<Self, BumpError> {
        match increment {
            IncrementCommand::Major => Ok(self.major_bump()),
            IncrementCommand::Minor => Ok(self.minor_bump()),
            IncrementCommand::Patch => Ok(self.patch_bump()),
            IncrementCommand::NoBump => Ok(self.no_bump()),
            IncrementCommand::Auto => {
                self.get_version_from_history(repository, None, |_, _| Ok(true))
            }
            IncrementCommand::AutoPackage(package) => self.get_version_from_history(
                repository,
                Some(&package),
                filter_commits_package(repository, &package),
            ),
            IncrementCommand::AutoMonoRepoGlobal(package_increment) => {
                self.auto_global_bump(repository, package_increment)
            }
            IncrementCommand::Manual(version) => self.manual_bump(&version).map_err(Into::into),
        }
    }

    pub(crate) fn reset_metadata(mut self) -> Self {
        self.version.build = BuildMetadata::EMPTY;
        self.version.pre = Prerelease::EMPTY;
        self.oid = None;
        self
    }

    fn get_version_from_history(
        &self,
        repository: &Repository,
        package: Option<&str>,
        filter: impl Fn(&OidOf, &Git2Commit) -> Result<bool, Git2Error>,
    ) -> Result<Tag, BumpError> {
        let oid = self
            .oid
            .or_else(|| {
                crate::get_cache(repository)
                    .get(&self.to_string())
                    .map(|oids| *oids[0].oid())
            })
            .unwrap_or_else(|| repository.get_first_commit().expect("empty repository"));

        let mut commits = Vec::new();
        for (oid, commit) in repository.revwalk_pkg(&format!("{oid}.."), package)? {
            if filter_merge_commits(&commit)
                && filter_fixup_commits(&commit)
                && filter(&oid, &commit)?
            {
                if let Ok(commit) = Commit::from_git_commit(&commit) {
                    commits.push(commit);
                }
            }
        }

        let increment_type = self.version_increment_from_commit_history(&commits)?;

        Ok(match increment_type {
            Increment::Major => self.major_bump(),
            Increment::Minor => self.minor_bump(),
            Increment::Patch => self.patch_bump(),
            Increment::NoBump => self.no_bump(),
        })
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

fn filter_commits_package<'a>(
    repository: &'a Repository,
    package: &'a str,
) -> impl Fn(&OidOf, &Git2Commit) -> Result<bool, Git2Error> + use<'a> {
    let package = SETTINGS.packages.get(package).expect("package exists");
    let path_filter = PackagePathFilter::from_package(package);
    let file_filter =
        move |file: git2::DiffFile| file.path().is_some_and(|path| path_filter.is_match(path));

    move |_, commit| {
        let parent = commit.parent(0).and_then(|commit| commit.tree()).ok();

        let diff = repository
            .0
            .diff_tree_to_tree(parent.as_ref(), Some(&commit.tree()?), None)?;

        Ok(diff
            .deltas()
            .any(|delta| file_filter(delta.old_file()) || file_filter(delta.new_file())))
    }
}

fn filter_commits_global(
    repository: &Repository,
) -> impl Fn(&OidOf, &Git2Commit) -> Result<bool, Git2Error> + use<'_> {
    let package_filter: Vec<_> = SETTINGS
        .packages
        .values()
        .map(PackagePathFilter::from_package)
        .collect();
    let file_filter = move |file: git2::DiffFile| {
        file.path()
            .is_some_and(|path| package_filter.iter().all(|filter| !filter.is_match(path)))
    };

    move |_, commit| {
        let Ok(parent) = commit.parent(0) else {
            return Ok(true);
        };

        let diff =
            repository
                .0
                .diff_tree_to_tree(Some(&parent.tree()?), Some(&commit.tree()?), None)?;

        Ok(diff
            .deltas()
            .any(|delta| file_filter(delta.old_file()) || file_filter(delta.new_file())))
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
        let repository = git_init_no_gpg()?;
        let base_version = Tag::from_str("1.0.0", None)?;

        // Act
        let tag = base_version.bump(IncrementCommand::Major, &repository)?;

        // Assert
        assert_that!(tag.version).is_equal_to(Version::new(2, 0, 0));
        Ok(())
    }

    #[sealed_test]
    fn minor_bump() -> Result<()> {
        // Arrange
        let repository = git_init_no_gpg()?;
        let base_version = Tag::from_str("1.0.0", None)?;

        // Act
        let tag = base_version.bump(IncrementCommand::Minor, &repository)?;

        // Assert
        assert_that!(tag.version).is_equal_to(Version::new(1, 1, 0));
        Ok(())
    }

    #[sealed_test]
    fn patch_bump() -> Result<()> {
        // Arrange
        let repository = git_init_no_gpg()?;
        let base_version = Tag::from_str("1.0.0", None)?;

        // Act
        let tag = base_version.bump(IncrementCommand::Patch, &repository)?;

        // Assert
        assert_that!(tag.version).is_equal_to(Version::new(1, 0, 1));
        Ok(())
    }

    #[sealed_test]
    fn no_bump() -> Result<()> {
        // Arrange
        let repository = git_init_no_gpg()?;
        let base_version = Tag::from_str("1.0.0", None)?;

        // Act
        let tag = base_version.bump(IncrementCommand::NoBump, &repository)?;

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
        let repository = git_init_no_gpg()?;
        let version = Tag::from_str("1.1.1", None)?;

        // Act
        let tag = version.bump(IncrementCommand::Minor, &repository)?;

        // Assert
        assert_that!(tag.version).is_equal_to(Version::from_str("1.2.0")?);

        Ok(())
    }

    #[sealed_test]
    fn increment_major_version_should_set_minor_and_patch_to_zero() -> Result<()> {
        // Arrange
        let repository = git_init_no_gpg()?;
        let version = Tag::from_str("1.1.1", None)?;

        // Act
        let tag = version.bump(IncrementCommand::Major, &repository)?;

        // Assert
        assert_that!(tag.version).is_equal_to(Version::from_str("2.0.0")?);

        Ok(())
    }

    #[sealed_test]
    fn increment_should_strip_metadata() -> Result<()> {
        // Arrange
        let repository = git_init_no_gpg()?;
        let version = Tag::from_str("1.1.1-pre+10.1", None)?;

        // Act
        let tag = version.bump(IncrementCommand::Patch, &repository)?;

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
        run_cmd!(
            echo "feature" > one;
            git add .;
            git commit -m "feat: feature package one";
        )?;
        git_tag("1.0.0")?;

        let base_version = Tag::from_str("1.0.0", None)?;

        // Act
        let tag = base_version.get_version_from_history(
            &repository,
            None,
            super::filter_commits_global(&repository),
        );

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
        let tag = base_version.auto_global_bump(&repository, Some(Increment::Minor))?;

        // Assert
        assert_that!(tag.version).is_equal_to(Version::new(0, 2, 0));

        Ok(())
    }

    #[sealed_test]
    fn monorepo_auto_bump_should_succeed_with_only_global_commits() -> Result<()> {
        // Arrange
        let repository = init_monorepo()?;

        run_cmd!(
            echo "global" > global;
            git add .;
            git commit -m "feat: non package commit";
        )?;

        // Act
        let tag = Tag::default().auto_global_bump(&repository, None)?;

        // Assert
        assert_that!(tag.version).is_equal_to(Version::new(0, 1, 0));

        Ok(())
    }

    #[sealed_test]
    fn monorepo_auto_bump_should_succeed_selecting_history_bump() -> Result<()> {
        // Arrange
        let repository = init_monorepo()?;

        // Patch increment from global commits
        // Minor increment from package bumps
        run_cmd!(
            echo "global" > global;
            git add .;
            git commit -m "fix: global fix";
            echo "feature" > one;
            git add .;
            git commit -m "feat: feature 1 package one";
        )?;

        // Act
        let tag = Tag::default().auto_global_bump(&repository, Some(Increment::Minor))?;

        // Assert
        assert_that!(tag.version).is_equal_to(Version::new(0, 1, 0));

        Ok(())
    }

    #[sealed_test]
    fn monorepo_auto_bump_should_succeed_selecting_package_bump() -> Result<()> {
        // Arrange
        let repository = init_monorepo()?;

        // Minor increment from global commits
        // Patch increment from package bumps
        run_cmd!(
            echo "global" > global;
            git add .;
            git commit -m "feat: global fix";
            echo "feature" > one;
            git add .;
            git commit -m "fix: fix 1 package one";
        )?;

        // Act
        let tag = Tag::default().auto_global_bump(&repository, Some(Increment::Patch))?;

        // Assert
        assert_that!(tag.version).is_equal_to(Version::new(0, 1, 0));

        Ok(())
    }

    #[sealed_test]
    fn monorepo_auto_bump_should_succeed_with_equals_history_and_package() -> Result<()> {
        // Arrange
        let repository = init_monorepo()?;

        // Minor increment from global commits
        // Minor increment from package bumps
        run_cmd!(
            echo "global" > global;
            git add .;
            git commit -m "feat: global fix";
            echo "feature" > one;
            git add .;
            git commit -m "feature: package one";
        )?;

        // Act
        let tag = Tag::default().auto_global_bump(&repository, Some(Increment::Minor))?;

        // Assert
        assert_that!(tag.version).is_equal_to(Version::new(0, 1, 0));

        Ok(())
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
        let tag =
            Tag::from_str("0.1.0", None)?.auto_global_bump(&repository, Some(Increment::Minor))?;

        // Assert
        assert_that!(tag.version).is_equal_to(Version::new(0, 2, 0));

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
        )?;

        Ok(repository)
    }
}
