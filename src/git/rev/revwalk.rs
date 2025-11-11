use git2::Commit;

use crate::git::error::Git2Error;
use crate::git::oid::CommitInfo;
use crate::git::repository::Repository;
use crate::git::rev::filters::CommitFilter;
use crate::git::rev::revspec::RevSpecPattern2;
use crate::git::rev::CommitIter;
use crate::git::tag::Tag;

impl Repository {
    /// Return a commit range from a [`RevSpecPattern2`]
    pub fn revwalk(&self, spec: RevSpecPattern2) -> Result<CommitIter<'_>, Git2Error> {
        let cache = self.get_cache();

        let mut revwalk = self.0.revwalk()?;
        if let Some(to) = spec.to {
            revwalk.push(to)?;
        } else {
            revwalk.push_head()?;
        }
        if let Some(from) = spec.from {
            revwalk.hide(from)?;
        }

        let mut commits: Vec<(CommitInfo, Commit)> = vec![];
        for oid in revwalk {
            let oid = oid?;
            let info = cache.get_info(oid);
            let commit = self.0.find_commit(oid)?;
            commits.push((info, commit));
        }

        Ok(CommitIter {
            commits,
            spec,
            repository: self,
            filter: CommitFilter::Standard,
        })
    }

    /// a revwalk for with the pattern `{tag}..`
    pub fn commits_since_tag(&self, tag: &Tag) -> Result<CommitIter<'_>, Git2Error> {
        let spec = RevSpecPattern2 {
            from: tag.oid.or_else(|| self.parse_rev(&tag.to_string()).ok()),
            to: None,
        };
        self.revwalk(spec)
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
    use crate::git::error::Git2Error;
    use crate::git::oid::CommitInfo;
    use crate::git::repository::Repository;
    use crate::git::rev::filters::CommitFilter;
    use crate::git::rev::revspec::RevSpecPattern2;
    use crate::git::rev::CommitIter;
    use crate::git::tag::Tag;
    use crate::settings::{MonoRepoPackage, Settings};
    use crate::test_helpers::{commit, git_init_no_gpg, git_tag, mkdir};

    const COCOGITTO_REPOSITORY: &str = env!("CARGO_MANIFEST_DIR");

    impl Repository {
        pub fn revwalk_pattern(&self, spec: &str) -> Result<CommitIter<'_>, Git2Error> {
            self.revwalk(self.revspec_from_str(spec)?)
        }
    }

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

        let range = repo.revwalk_pattern("0.1.0");

        let range = range?;

        // Act
        let release = Release::build_multi(&range)?;

        // Assert
        assert_that!(release).has_length(1);
        assert_that!(release[0].version.id).is_equal_to(&Oid::from_str(&three)?);
        assert_that!(release[0].from.id).is_equal_to(&Oid::from_str(&one)?);

        let expected_commits: Vec<_> = release[0]
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
        let release = Release::build_multi(&range)?;

        // Assert
        assert_that!(release.get(1)).is_some().matches(|child| {
            let commits: Vec<String> = child
                .commits
                .iter()
                .map(|commit| commit.commit.oid.clone())
                .collect();

            commits == [three.clone(), two.clone(), one.clone()]
        });

        assert_that!(release[0].version.to_string()).is_equal_to("0.2.0".to_string());
        assert_that!(release[0].from.to_string()).is_equal_to("0.1.0".to_string());

        let expected_commits: Vec<String> = release[0]
            .commits
            .iter()
            .map(|commit| commit.commit.oid.clone())
            .collect();

        assert_that!(expected_commits).is_equal_to(vec![five, four]);

        Ok(())
    }

    #[test]
    fn get_release_range_integration_test() -> Result<()> {
        // Arrange
        let repo = Repository::open(COCOGITTO_REPOSITORY)?;
        let range = repo.revwalk_pattern("0.32.1..0.32.3")?;

        // Act
        let releases = Release::build_multi(&range)?;

        // Assert
        assert_that!(releases).has_length(2);
        let [release_1, release_2] = releases.try_into().expect("checked length");
        assert_that!(release_1.version.to_string()).is_equal_to("0.32.3".to_string());
        assert_that!(release_1.from.to_string()).is_equal_to("0.32.2".to_string());

        assert_that!(release_2.version.to_string()).is_equal_to("0.32.2".to_string());
        assert_that!(release_2.from.to_string()).is_equal_to("0.32.1".to_string());

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
        let mut info = CommitInfo::new(Oid::from_str(&first)?);
        info.first = true;
        assert_that!(commit_range.from_oid()).is_equal_to(info);
        assert_that!(commit_range.to_oid().to_string()).is_equal_to("1.0.0".to_string());
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

        mkdir(&["one"])?;

        run_cmd!(
            echo $settings > cog.toml;
            git add .;
            git commit -m "chore: First commit";
            echo changes > one/file;
            git add .;
            git commit -m "feat: package one";
            echo changes > global;
            git add .;
            git commit -m "feat: global change";
        )?;

        let commit_range_package = repo
            .revwalk(RevSpecPattern2::full())?
            .apply_filter(CommitFilter::Package("one"))?;
        let commit_range_global = repo
            .revwalk(RevSpecPattern2::full())?
            .apply_filter(CommitFilter::Global)?;
        assert_that!(commit_range_package.commits).has_length(1);
        assert_that!(commit_range_global.commits).has_length(2);
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

        let commit_range_package = repo
            .revwalk_pattern("HEAD~1..HEAD")?
            .apply_filter(CommitFilter::Package("one"))?;

        asserting!("package with changes have some commits")
            .that(&commit_range_package.commits)
            .mapped_contains(
                |(_, commit)| commit.message(),
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

        let commit_range_package = repo
            .revwalk_pattern("HEAD~1..HEAD")?
            .apply_filter(CommitFilter::Package("one"))?;

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

        let commit_range_package = repo
            .revwalk_pattern("HEAD~1..HEAD")?
            .apply_filter(CommitFilter::Package("one"))?;

        asserting!("package with shared changes should have some commits for HEAD~1 revspec")
            .that(&commit_range_package.commits)
            .mapped_contains(
                |(_, commit)| commit.message(),
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

        let commit_range_package = repo
            .revwalk_pattern("HEAD~1..HEAD")?
            .apply_filter(CommitFilter::Package("one"))?;

        asserting!(
            "package with extra included changes should have some commits for HEAD~1 revspec"
        )
        .that(&commit_range_package.commits)
        .mapped_contains(
            |(_, commit)| commit.message(),
            &Some("feat: commit to extra included file\n"),
        );

        assert_that!(commit_range_package.commits).has_length(1);

        Ok(())
    }

    #[sealed_test]
    fn package_with_with_only_ignored_path_should_have_no_commit() -> Result<()> {
        let repo = init_mono_repo_for_range_filtering()?;
        let commit_range_package = repo
            .revwalk(RevSpecPattern2::full())?
            .apply_filter(CommitFilter::Package("one"))?;
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

        mkdir(&["shared", "one/ignored"])?;

        run_cmd!(
            echo $settings > cog.toml;
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
        assert_that!(commit_range.from_oid().to_string()).is_equal_to(first);
        assert_that!(commit_range.to_oid().to_string()).is_equal_to("1.0.0".to_string());
        assert_that!(commit_range.commits).has_length(2);
        Ok(())
    }

    #[test]
    fn from_tag_to_tag_ok() -> Result<()> {
        // Arrange
        let repo = Repository::open(COCOGITTO_REPOSITORY)?;
        let v1_0_0 = Oid::from_str("549070fa99986b059cbaa9457b6b6f065bbec46b")?;
        let _v1_0_0 = CommitInfo::from(Tag::from_str("1.0.0", Some(v1_0_0))?);
        let v3_0_0 = Oid::from_str("c6508e243e2816e2d2f58828ee0c6721502958dd")?;
        let v3_0_0 = CommitInfo::from(Tag::from_str("3.0.0", Some(v3_0_0))?);

        // Act
        let range = repo.revwalk_pattern("1.0.0..3.0.0")?;

        // Assert
        assert_that!(range.from_oid().to_string())
            .is_equal_to("7c4a1cb692b445f36a44857ce17db32b91acd24c".to_string());
        assert_that!(range.to_oid()).is_equal_to(v3_0_0);

        Ok(())
    }

    #[test]
    fn from_tag_to_head() -> Result<()> {
        // Arrange
        let repo = Repository::open(COCOGITTO_REPOSITORY)?;
        let head = repo.get_head_commit_oid()?;
        let mut head = CommitInfo::new(head);
        head.head = true;
        let tag = repo.get_latest_tag(None, false)?;

        // Cover the case when we release a version and run the test in the CI right after that
        let head = if tag.oid == Some(head.oid) {
            CommitInfo::from(tag)
        } else {
            head
        };

        let v1_0_0 = Oid::from_str("549070fa99986b059cbaa9457b6b6f065bbec46b")?;
        let _v1_0_0 = CommitInfo::from(Tag::from_str("1.0.0", Some(v1_0_0))?);

        // Act
        let range = repo.revwalk_pattern("1.0.0..")?;

        // Assert
        assert_that!(range.from_oid().oid)
            .is_equal_to(&Oid::from_str("7c4a1cb692b445f36a44857ce17db32b91acd24c")?);
        assert_that!(range.to_oid()).is_equal_to(head);

        Ok(())
    }

    #[test]
    fn from_previous_to_tag() -> Result<()> {
        // Arrange
        let repo = Repository::open(COCOGITTO_REPOSITORY)?;
        let v2_1_1 = Oid::from_str("9dcf728d2eef6b5986633dd52ecbe9e416234898")?;
        let _v2_1_1 = CommitInfo::from(Tag::from_str("2.1.1", Some(v2_1_1))?);
        let v3_0_0 = Oid::from_str("c6508e243e2816e2d2f58828ee0c6721502958dd")?;
        let v3_0_0 = CommitInfo::from(Tag::from_str("3.0.0", Some(v3_0_0))?);

        // Act
        let range = repo.revwalk_pattern("2.1.1..3.0.0")?;

        // Assert
        assert_that!(range.from_oid().oid)
            .is_equal_to(&Oid::from_str("434c22295390fda0f276e3a3ee32fa4658489c5d")?);
        assert_that!(range.to_oid()).is_equal_to(v3_0_0);

        Ok(())
    }

    #[test]
    fn recursive_from_origin_to_head() -> Result<()> {
        // Arrange
        let repo = Repository::open(COCOGITTO_REPOSITORY)?;
        let cache = repo.get_cache();
        let mut tags = cache.tags.clone();
        tags.sort();
        let mut tag_count = tags.len();
        // if head doesn't have a tag, one additional "unreleased" release is generated
        if cache.get_info(cache.head).tags.is_empty() {
            tag_count += 1;
        }
        drop(cache);

        let range = repo.revwalk_pattern("..")?;

        // Act
        let releases = Release::build_multi(&range)?;

        // Assert
        assert_that!(releases).has_length(tag_count);
        for (tag, release) in tags.into_iter().zip(releases.into_iter().rev()) {
            assert_that!(release.version).is_equal_to(&tag.into());
        }

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
        let release = Release::build(&range)?;

        // Assert
        let actual_oids: Vec<String> = release
            .commits
            .iter()
            .map(|commit| commit.commit.oid.to_string())
            .collect();

        assert_that!(actual_oids).is_equal_to(vec![four, three, two]);

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
        let releases = Release::build_multi(&range)?;
        let [release_1, release_2] = releases.try_into().expect("should be 2 releases");

        // Assert
        let head_to_v1: Vec<String> = release_1
            .commits
            .iter()
            .map(|commit| commit.commit.oid.to_string())
            .collect();

        let commit_before_v1: Vec<String> = release_2
            .commits
            .iter()
            .map(|commit| commit.commit.oid.to_string())
            .collect();

        assert_that!(head_to_v1).is_equal_to(vec![three]);
        assert_that!(commit_before_v1).is_equal_to(vec![two, one]);

        Ok(())
    }

    #[sealed_test]
    fn get_repo_packages() -> Result<()> {
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

        mkdir(&["one", "two"])?;

        run_cmd!(
            echo "one" > one/file;
            git add .;
            git commit -m "feat: package one";
            echo "two" > two/file;
            git add .;
            git commit -m "feat: package two";
            echo "two" > two/file2;
            git add .;
            git commit -m "feat: more changes to two";
        )?;

        // Act
        let range = repo
            .revwalk(RevSpecPattern2::full())?
            .apply_filter(CommitFilter::Package("two"))?;
        let range = range.into_iter().collect::<Vec<_>>();

        // Assert
        assert_that!(range).has_length(2);

        Ok(())
    }
}
