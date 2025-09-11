use std::collections::HashMap;

use crate::git::error::Git2Error;
use crate::git::repository::Repository;
use crate::git::rev::revspec::RevSpec;
use crate::git::rev::CommitRange;

pub(crate) type RevwalkCache<'c> = HashMap<RevSpec, CommitRange<'c>>;

impl Repository {
    pub fn revwalk_pattern(&self, pattern: &str) -> Result<CommitRange<'_>, Git2Error> {
        let spec = RevSpec::parse(pattern, self)?;
        self.revwalk(&spec)
    }

    pub fn revwalk(&self, spec: &RevSpec) -> Result<CommitRange<'_>, Git2Error> {
        let mut revwalk = self.0.revwalk()?;
        if let Some(from) = spec.from {
            revwalk.hide(from)?;
        }
        if let Some(to) = spec.to {
            revwalk.push(to)?;
        } else {
            revwalk.push_head()?;
        }
        Ok(CommitRange {
            commits: revwalk
                .map(|oid| oid.and_then(|oid| self.0.find_commit(oid)))
                .collect::<Result<_, _>>()?,
            repository: self,
        })
    }

    pub fn revwalk_cached<'c>(
        &'c self,
        spec: &'_ RevSpec,
        cache: &'_ mut RevwalkCache<'c>,
    ) -> Result<CommitRange<'c>, Git2Error> {
        if let Some(cached) = cache.get(spec) {
            Ok(cached.clone())
        } else {
            let res = self.revwalk(spec)?;
            cache.insert(*spec, res.clone());
            Ok(res)
        }
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use anyhow::Result;

    use cmd_lib::run_cmd;
    use git2::Oid;
    use indoc::formatdoc;
    use sealed_test::prelude::*;
    use speculoos::prelude::*;

    use crate::conventional::changelog::release::Release;
    use crate::git::error::TagError;
    use crate::git::repository::Repository;
    use crate::git::rev::cache::CommitInfo;
    use crate::git::rev::filters::CommitRangeFilter;
    use crate::git::tag::Tag;
    use crate::settings::{MonoRepoPackage, Settings};
    use crate::test_helpers::{commit, git_init_no_gpg, git_tag};

    const COCOGITTO_REPOSITORY: &str = env!("CARGO_MANIFEST_DIR");

    #[test]
    fn all_commits() -> Result<()> {
        // Arrange
        let repo = Repository::open(COCOGITTO_REPOSITORY)?;

        // Act
        let range = repo.revwalk_pattern("..")?;

        // Assert
        assert_that!(range.commits).is_not_empty();
        Ok(())
    }

    #[sealed_test]
    fn should_get_range_for_a_single_release() -> Result<()> {
        // Arrange
        let repo = git_init_no_gpg()?;
        let one = commit("chore: first commit")?;
        let two = commit("feat: feature 1")?;
        let three = commit("feat: feature 2")?;
        git_tag("0.1.0")?;

        let range = repo.revwalk_pattern("0.1.0")?;

        // Act
        let releases = Release::build(range, CommitRangeFilter::None);
        let release = &releases[0];

        // Assert
        assert_that!(releases.len()).is_equal_to(1);
        assert_that!(release.version.id).is_equal_to(&Oid::from_str(&three)?);
        assert_that!(release.from).is_equal_to(&Oid::from_str(&one)?.into());

        let expected_commits: Vec<_> = release
            .commits
            .iter()
            .map(|commit| &commit.commit.oid)
            .collect();

        assert_that!(expected_commits).is_equal_to(vec![&three, &two, &one]);

        Ok(())
    }

    #[sealed_test]
    fn should_get_range_for_a_multiple_release() -> Result<()> {
        // Arrange
        let repo = git_init_no_gpg()?;
        let one = commit("chore: first commit")?;
        let two = commit("feat: feature 1")?;
        let three = commit("feat: feature 2")?;
        git_tag("0.1.0")?;
        let four = commit("feat: feature 3")?;
        let five = commit("feat: feature 4")?;
        git_tag("0.2.0")?;

        let range = repo.revwalk_pattern("..0.2.0")?;

        // Act
        let releases = Release::build(range, CommitRangeFilter::None);

        // Assert
        assert_that!(releases.get(1)).is_some().matches(|v1| {
            let commits: Vec<_> = v1.commits.iter().map(|commit| &commit.commit.oid).collect();

            commits == [&three, &two, &one]
        });

        assert_that!(releases[0].version.to_string()).is_equal_to("0.2.0".to_string());
        assert_that!(releases[0].from.to_string()).is_equal_to("0.1.0".to_string());

        let expected_commits: Vec<_> = releases[0]
            .commits
            .iter()
            .map(|commit| &commit.commit.oid)
            .collect();

        assert_that!(expected_commits).is_equal_to(vec![&five, &four]);

        Ok(())
    }

    #[test]
    fn get_release_range_integration_test() -> Result<()> {
        // Arrange
        let repo = Repository::open(COCOGITTO_REPOSITORY)?;
        let range = repo.revwalk_pattern("0.32.1..0.32.3")?;

        // Act
        let releases = Release::build(range, CommitRangeFilter::None);

        // Assert
        assert_that!(releases[0].version.to_string()).is_equal_to("0.32.3".to_string());

        assert_that!(releases[1].version.to_string()).is_equal_to("0.32.2".to_string());

        assert_that!(releases).has_length(2);
        Ok(())
    }

    #[sealed_test]
    fn get_annotated_tag_commits() -> Result<()> {
        // Arrange
        let repo = git_init_no_gpg()?;
        let first = commit("chore: init")?;
        let _second = commit("chore: 1.0.0")?;

        // Create an annotated tag
        let head = repo.get_head_commit()?;
        let sig = git2::Signature::now("Author", "email@example.com")?;
        repo.0
            .tag("1.0.0", &head.into_object(), &sig, "the_msg", false)?;

        commit("feat: a commit")?;

        // Act
        let commit_range = repo.revwalk_pattern("..1.0.0")?;

        // Assert
        assert_that!(commit_range.from()).is_equal_to(CommitInfo {
            oid: Oid::from_str(&first)?,
            tags: Vec::new(),
        });
        assert_that!(commit_range.to().to_string()).is_equal_to("1.0.0".to_string());
        assert_that!(commit_range.commits).has_length(2);
        Ok(())
    }

    #[sealed_test]
    fn get_package_commit_range() -> Result<()> {
        // Arrange
        let repo = git_init_no_gpg()?;
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
            git commit -m "chore: First commit";
            mkdir one;
            echo changes > one/file;
            git add .;
            git commit -m "feat: package one";
            echo changes > global;
            git add .;
            git commit -m "feat: global change";
        )?;

        let global_range = repo.revwalk_pattern("..HEAD")?.global()?;
        let package_range = repo.revwalk_pattern("..HEAD")?.package("one")?;

        assert_that!(package_range.commits).has_length(1);
        assert_that!(global_range.commits).has_length(2);
        Ok(())
    }

    #[sealed_test]
    fn package_with_changes_have_some_commits() -> Result<()> {
        let repo = init_mono_repo_for_range_filtering()?;

        run_cmd!(
            echo changes > one/file;
            git add .;
            git commit -m "feat: commit to non-ignored path";
        )?;

        let commit_range_package = repo.revwalk_pattern("HEAD~1..HEAD")?.package("one")?;

        asserting!("package with changes have some commits")
            .that(&commit_range_package.commits)
            .mapped_contains(
                |commit| commit.message(),
                &Some("feat: commit to non-ignored path\n"),
            );

        Ok(())
    }

    #[sealed_test]
    fn package_with_no_changes_should_have_no_commit() -> Result<()> {
        let repo = init_mono_repo_for_range_filtering()?;

        run_cmd!(
            echo changes > one/ignored/file;
            git add .;
            git commit -m "feat: commit to ignored path";
        )?;

        let commit_range_package = repo.revwalk_pattern("HEAD~1..HEAD")?.package("one")?;

        asserting!("package with no changes should have some commits for HEAD~1 revspec")
            .that(&commit_range_package.commits)
            .has_length(0);

        Ok(())
    }

    #[sealed_test]
    fn package_with_shared_changes_should_have_some_commits() -> Result<()> {
        let repo = init_mono_repo_for_range_filtering()?;

        run_cmd!(
            echo changes > shared/file;
            git add .;
            git commit -m "feat: commit to extra included path";
        )?;

        let commit_range_package = repo.revwalk_pattern("HEAD~1..HEAD")?.package("one")?;

        asserting!("package with shared changes should have some commits for HEAD~1 revspec")
            .that(&commit_range_package.commits)
            .mapped_contains(
                |commit| commit.message(),
                &Some("feat: commit to extra included path\n"),
            );

        assert_that!(commit_range_package.commits).has_length(1);

        Ok(())
    }

    #[sealed_test]
    fn package_with_extra_included_changes_should_have_some_commits() -> Result<()> {
        let repo = init_mono_repo_for_range_filtering()?;

        run_cmd!(
            echo changes > README.md;
            git add .;
            git commit -m "feat: commit to extra included file";
        )?;

        let commit_range_package = repo.revwalk_pattern("HEAD~1..HEAD")?.package("one")?;

        asserting!(
            "package with extra included changes should have some commits for HEAD~1 revspec"
        )
        .that(&commit_range_package.commits)
        .mapped_contains(
            |commit| commit.message(),
            &Some("feat: commit to extra included file\n"),
        );

        assert_that!(commit_range_package.commits).has_length(1);

        Ok(())
    }

    #[sealed_test]
    fn package_with_with_only_ignored_path_should_have_no_commit() -> Result<()> {
        let repo = init_mono_repo_for_range_filtering()?;
        let commit_range_package = repo.revwalk_pattern("..HEAD")?.package("one")?;
        asserting!("package with with only ignored path commit should have no commits for full range revspec")
            .that(&commit_range_package.commits).has_length(0);

        Ok(())
    }

    fn init_mono_repo_for_range_filtering() -> Result<Repository> {
        let repo = git_init_no_gpg()?;
        let mut packages = HashMap::new();
        packages.insert(
            "one".to_string(),
            MonoRepoPackage {
                path: PathBuf::from("one"),
                include: vec![String::from("shared/**"), String::from("README.md")],
                ignore: vec![String::from("one/ignored/**")],
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
            mkdir -p shared one/ignored;
            git add .;
            git commit -m "chore: First commit";
        )?;
        Ok(repo)
    }

    #[sealed_test]
    fn get_tag_commits() -> Result<()> {
        // Arrange
        let repo = git_init_no_gpg()?;

        let first = commit("chore: init")?;
        commit("chore: 1.0.0")?;
        git_tag("1.0.0")?;
        commit("feat: a commit")?;

        // Act
        let commit_range = repo.revwalk_pattern("..1.0.0")?;

        // Assert
        assert_that!(commit_range.from().to_string()).is_equal_to(first);
        assert_that!(commit_range.to().to_string()).is_equal_to("1.0.0".to_string());
        assert_that!(commit_range.commits).has_length(2);
        Ok(())
    }

    #[test]
    fn from_tag_to_tag_ok() -> Result<()> {
        // Arrange
        let repo = Repository::open(COCOGITTO_REPOSITORY)?;
        let v3_0_0 = Oid::from_str("c6508e243e2816e2d2f58828ee0c6721502958dd")?;
        let v3_0_0 = CommitInfo::from_tag(&Tag::from_str("3.0.0", Some(v3_0_0))?);

        // Act
        let range = repo.revwalk_pattern("1.0.0..3.0.0")?;

        // Assert
        assert_that!(range.from().to_string())
            .is_equal_to("7c4a1cb692b445f36a44857ce17db32b91acd24c".to_string());
        assert_that!(range.to()).is_equal_to(v3_0_0);

        Ok(())
    }

    #[test]
    fn from_tag_to_head() -> Result<()> {
        // Arrange
        let repo = Repository::open(COCOGITTO_REPOSITORY)?;
        let head = repo.get_cache().head;
        let head_info = repo.get_cache().get_info(head);

        // Act
        let range = repo.revwalk_pattern("1.0.0..")?;

        // Assert
        assert_that!(range.from().oid)
            .is_equal_to(&Oid::from_str("7c4a1cb692b445f36a44857ce17db32b91acd24c")?);
        assert_that!(range.to()).is_equal_to(head_info);

        Ok(())
    }

    #[test]
    fn from_previous_to_tag() -> Result<()> {
        // Arrange
        let repo = Repository::open(COCOGITTO_REPOSITORY)?;
        let v3_0_0 = Oid::from_str("c6508e243e2816e2d2f58828ee0c6721502958dd")?;
        let v3_0_0 = CommitInfo::from_tag(&Tag::from_str("3.0.0", Some(v3_0_0))?);

        // Act
        let range = repo.revwalk_pattern("2.1.1..3.0.0")?;

        // Assert
        assert_that!(range.from().oid)
            .is_equal_to(&Oid::from_str("434c22295390fda0f276e3a3ee32fa4658489c5d")?);
        assert_that!(range.to()).is_equal_to(v3_0_0);

        Ok(())
    }

    #[test]
    fn recursive_from_origin_to_head() -> Result<()> {
        // Arrange
        let repo = Repository::open(COCOGITTO_REPOSITORY)?;
        let mut tag_count = repo.0.tag_names(None)?.len();
        let head = repo.get_cache().head;
        let latest = repo.get_latest_tag(None, false).ok_or(TagError::NoTag)?;
        if latest.oid != Some(head) {
            tag_count += 1;
        };

        let range = repo.revwalk_pattern("..")?;

        // Act
        let releases = Release::build(range, CommitRangeFilter::None);

        // Assert
        assert_that!(releases).has_length(tag_count);

        Ok(())
    }

    #[sealed_test]
    fn from_commit_to_head() -> Result<()> {
        // Arrange
        let repo = git_init_no_gpg()?;

        commit("chore: init")?;
        commit("feat: a commit")?;
        let one = commit("chore: another commit")?;
        let two = commit("feat: a feature")?;
        let three = commit("chore: 1.0.0")?;
        let four = commit("fix: the bug")?;

        let range = repo.revwalk_pattern(&format!("{}..", &one[0..7]))?;

        // Act
        let release = Release::build(range, CommitRangeFilter::None);

        // Assert
        let actual_oids: Vec<_> = release[0]
            .commits
            .iter()
            .map(|commit| &commit.commit.oid)
            .collect();

        assert_that!(actual_oids).is_equal_to(vec![&four, &three, &two]);

        Ok(())
    }

    #[sealed_test]
    fn from_commit_to_head_with_overlapping_tag() -> Result<()> {
        // Arrange
        let repo = git_init_no_gpg()?;

        commit("chore: init")?;
        commit("feat: a commit")?;

        let from = commit("chore: another commit")?;
        let one = commit("feat: a feature")?;
        let two = commit("chore: 1.0.0")?;
        git_tag("1.0.0")?;
        let three = commit("fix: the bug")?;

        let range = repo.revwalk_pattern(&format!("{}..", &from[0..7]))?;

        // Act
        let releases = Release::build(range, CommitRangeFilter::None);

        // Assert
        let head_to_v1: Vec<_> = releases[0]
            .commits
            .iter()
            .map(|commit| &commit.commit.oid)
            .collect();

        let commit_before_v1: Vec<_> = releases[1]
            .commits
            .iter()
            .map(|commit| &commit.commit.oid)
            .collect();

        assert_that!(head_to_v1).is_equal_to(vec![&three]);
        assert_that!(commit_before_v1).is_equal_to(vec![&two, &one]);

        Ok(())
    }

    #[sealed_test]
    fn get_package_commits() -> Result<()> {
        // Arrange
        let settings = formatdoc!(
            "
            [packages.one]
            path = \"one\"
            changelog_path = \"one/CHANGELOG.md\"

            [packages.two]
            path = \"two\"
            changelog_path = \"two/CHANGELOG.md\"
            "
        );

        let repo = git_init_no_gpg()?;
        run_cmd!(
            echo $settings > cog.toml;
            git add .;
        )?;

        repo.commit("chore: init", false, false)?;

        run_cmd!(
            mkdir one;
            echo "one" > one/file;
            git add .;
            git commit -m "feat: package one";
            mkdir two;
            echo "two" > two/file;
            git add .;
            git commit -m "feat: package two";
            echo "two" > two/file2;
            git add .;
            git commit -m "feat: more changes to two";
        )?;

        // Act
        let range = repo.revwalk_pattern("..HEAD")?.package("two")?;

        // Assert
        assert_that!(range.commits).has_length(2);

        Ok(())
    }
}
