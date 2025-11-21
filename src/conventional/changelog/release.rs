use crate::conventional::commit::Commit;
use crate::git::commit::GitCommitExt;
use crate::git::oid::ReleaseVersion;
use crate::git::rev::CommitIter;
use crate::settings;
use crate::{conventional::changelog::error::ChangelogError, Target};
use chrono::{NaiveDateTime, Utc};
use colored::Colorize;
use conventional_commit_parser::commit::Footer;
use log::warn;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct Release<'a> {
    pub version: ReleaseVersion,
    pub from: ReleaseVersion,
    pub date: NaiveDateTime,
    pub commits: Vec<ChangelogCommit<'a>>,
    pub packages: Vec<PackageBump<'a>>,
    #[serde(skip)]
    pub target: Target,
}

#[derive(Debug, Serialize, Clone)]
pub struct PackageBump<'a> {
    pub package_name: &'a str,
    pub package_path: &'a Path,
    pub version: ReleaseVersion,
    pub from: Option<ReleaseVersion>,
}

impl Release<'static> {
    pub fn build(commits: &CommitIter<'_>) -> Result<Self, ChangelogError> {
        if commits.is_empty() {
            Err(ChangelogError::EmptyRelease)
        } else {
            Ok(Self::build_raw(commits))
        }
    }

    pub(crate) fn build_raw(commits: &CommitIter<'_>) -> Self {
        let (from, version) = commits.version_range();
        Release {
            version,
            from,
            date: Utc::now().naive_local(),
            commits: commits
                .iter_commits()
                .filter(|commit| commit.message().is_some())
                .filter(|commit| commit.should_retain_by_settings())
                .filter_map(|commit| {
                    Commit::from_git_commit(commit)
                        .inspect_err(|err| warn!("{}", err.to_string().red()))
                        .ok()
                })
                .filter(|commit| !commit.should_omit())
                .map(ChangelogCommit::from)
                .collect(),
            packages: Vec::new(),
            target: commits.target,
        }
    }

    pub fn build_multi(commits: &CommitIter<'_>) -> Result<Vec<Self>, ChangelogError> {
        let mut res = commits
            .split_at_tags()?
            .iter()
            .map(Self::build_raw)
            .collect::<Vec<_>>();
        res.reverse();
        Ok(res)
    }
}

#[derive(Debug)]
pub struct ChangelogCommit<'a> {
    pub author_username: Option<&'a str>,
    pub commit: Commit,
}

impl From<Commit> for ChangelogCommit<'_> {
    fn from(commit: Commit) -> Self {
        let author_username = settings::commit_username(&commit.author);

        ChangelogCommit {
            author_username,
            commit,
        }
    }
}

#[derive(Serialize)]
pub struct ChangelogFooter<'a> {
    token: &'a str,
    content: &'a str,
}

impl<'a> From<&'a Footer> for ChangelogFooter<'a> {
    fn from(footer: &'a Footer) -> Self {
        Self {
            token: footer.token.as_str(),
            content: footer.content.as_str(),
        }
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use anyhow::Result;
    use chrono::NaiveDateTime;
    use conventional_commit_parser::commit::{CommitType, ConventionalCommit, Footer};
    use git2::Oid;
    use indoc::indoc;
    use itertools::Itertools;
    use pretty_assertions::assert_eq;
    use speculoos::prelude::*;

    use crate::conventional::changelog::release::{ChangelogCommit, PackageBump, Release};
    use crate::conventional::changelog::renderer::Renderer;
    use crate::conventional::changelog::template::{PredefinedTemplate, RemoteContext, Template};
    use crate::conventional::commit::Commit;
    use crate::git::repository::Repository;
    use crate::Target;

    use crate::git::rev::revspec::RevSpecPattern2;
    use crate::git::tag::Tag;

    macro_rules! assert_doc_eq {
        ($changelog:expr, $doc:literal) => {
            assert_eq!(
                $changelog.split('\n').map(|line| line.trim()).join("\n"),
                indoc!($doc).split('\n').map(|line| line.trim()).join("\n")
            )
        };
    }

    #[test]
    fn should_get_a_release() -> anyhow::Result<()> {
        let repo = Repository::open(".")?;
        let iter = repo.revwalk(RevSpecPattern2::full())?;
        let release = Release::build(&iter);
        assert_that!(release)
            .is_ok()
            .matches(|r| !r.commits.is_empty());
        Ok(())
    }

    #[test]
    fn should_render_default_template() -> Result<()> {
        // Arrange
        let release = Release::fixture(Target::Standard);
        let renderer = Renderer::default();

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_doc_eq!(
            changelog,
            "## 1.0.0 - 2015-09-05
            #### Features
            - (**parser**) implement the changelog generator - (17f7e23) - *oknozor*
            - awesome feature - (17f7e23) - Paul Delafosse
            #### Bug Fixes
            - (**parser**) fix parser implementation - (17f7e23) - *oknozor*
            "
        );

        Ok(())
    }

    #[test]
    fn should_render_full_hash_template() -> Result<()> {
        // Arrange
        let release = Release::fixture(Target::Standard);
        let renderer = Renderer::try_new(Template::Predefined(PredefinedTemplate::FULL_HASH))?;

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_doc_eq!(
            changelog,
            "#### Features
            - 17f7e23081db15e9318aeb37529b1d473cf41cbe - (**parser**) implement the changelog generator - @oknozor
            - 17f7e23081db15e9318aeb37529b1d473cf41cbe - awesome feature - Paul Delafosse
            #### Bug Fixes
            - 17f7e23081db15e9318aeb37529b1d473cf41cbe - (**parser**) fix parser implementation - @oknozor
            "
        );

        Ok(())
    }

    #[test]
    fn should_render_github_template() -> Result<()> {
        // Arrange
        let release = Release::fixture(Target::Standard);
        let mut renderer = Renderer::try_new(Template::Predefined(PredefinedTemplate::REMOTE))?;
        renderer.add_context(RemoteContext::fixture());

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_doc_eq!(
            changelog,
            "## [1.0.0](https://github.com/cocogitto/cocogitto/compare/0.1.0..1.0.0) - 2015-09-05
            #### Features
            - (**parser**) implement the changelog generator - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - [@oknozor](https://github.com/oknozor)
            - awesome feature - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - Paul Delafosse
            #### Bug Fixes
            - (**parser**) fix parser implementation - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - [@oknozor](https://github.com/oknozor)
            "
        );

        Ok(())
    }

    #[test]
    fn should_render_template_monorepo() -> Result<()> {
        // Arrange
        let release = Release::fixture_monorepo();
        let renderer =
            Renderer::try_new(Template::Predefined(PredefinedTemplate::MONOERPO_DEFAULT))?;

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_doc_eq!(
            changelog,
            "## 1.0.0 - 2015-09-05
            ### Package updates
            - one bumped to 0.1.0
            - two bumped to 0.2.0
            ### Global changes
            #### Features
            - (**parser**) implement the changelog generator - (17f7e23) - *oknozor*
            - awesome feature - (17f7e23) - Paul Delafosse
            #### Bug Fixes
            - (**parser**) fix parser implementation - (17f7e23) - *oknozor*
            "
        );

        Ok(())
    }

    #[test]
    fn should_render_template_monorepo_for_manual_bump() -> Result<()> {
        // Arrange
        let release = Release::fixture_manual();
        let renderer =
            Renderer::try_new(Template::Predefined(PredefinedTemplate::MONOERPO_DEFAULT))?;

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_doc_eq!(
            changelog,
            "## 1.0.0 - 2015-09-05
            ### Packages
            - one locked to 0.1.0
            - two locked to 0.2.0
            ### Global changes
            #### Features
            - (**parser**) implement the changelog generator - (17f7e23) - *oknozor*
            - awesome feature - (17f7e23) - Paul Delafosse
            #### Bug Fixes
            - (**parser**) fix parser implementation - (17f7e23) - *oknozor*
            "
        );

        Ok(())
    }

    #[test]
    fn should_render_full_hash_template_monorepo() -> Result<()> {
        // Arrange
        let release = Release::fixture_monorepo();
        let renderer =
            Renderer::try_new(Template::Predefined(PredefinedTemplate::MONOERPO_FULL_HASH))?;

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_doc_eq!(
            changelog,
            "### Package updates
            - one bumped to 0.1.0
            - two bumped to 0.2.0
            ### Global changes
            #### Features
            - 17f7e23081db15e9318aeb37529b1d473cf41cbe - (**parser**) implement the changelog generator - @oknozor
            - 17f7e23081db15e9318aeb37529b1d473cf41cbe - awesome feature - Paul Delafosse
            #### Bug Fixes
            - 17f7e23081db15e9318aeb37529b1d473cf41cbe - (**parser**) fix parser implementation - @oknozor
            "
        );

        Ok(())
    }

    #[test]
    fn should_render_full_hash_template_manual_monorepo() -> Result<()> {
        // Arrange
        let release = Release::fixture_manual();
        let renderer =
            Renderer::try_new(Template::Predefined(PredefinedTemplate::MONOERPO_FULL_HASH))?;

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_doc_eq!(
            changelog,
            "### Packages
            - one locked to 0.1.0
            - two locked to 0.2.0
            ### Global changes
            #### Features
            - 17f7e23081db15e9318aeb37529b1d473cf41cbe - (**parser**) implement the changelog generator - @oknozor
            - 17f7e23081db15e9318aeb37529b1d473cf41cbe - awesome feature - Paul Delafosse
            #### Bug Fixes
            - 17f7e23081db15e9318aeb37529b1d473cf41cbe - (**parser**) fix parser implementation - @oknozor
            "
        );

        Ok(())
    }

    #[test]
    fn should_render_remote_template_monorepo() -> Result<()> {
        // Arrange
        let release = Release::fixture_monorepo();
        let mut renderer =
            Renderer::try_new(Template::Predefined(PredefinedTemplate::MONOERPO_REMOTE))?;
        renderer.add_context(RemoteContext::fixture());

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_doc_eq!(
            changelog,
            "## [1.0.0](https://github.com/cocogitto/cocogitto/compare/0.1.0..1.0.0) - 2015-09-05
            ### Package updates
            - [one](crates/one) bumped to [0.1.0](https://github.com/cocogitto/cocogitto/compare/0.2.0..0.1.0)
            - [two](crates/two) bumped to [0.2.0](https://github.com/cocogitto/cocogitto/compare/0.3.0..0.2.0)
            ### Global changes
            #### Features
            - (**parser**) implement the changelog generator - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - [@oknozor](https://github.com/oknozor)
            - awesome feature - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - Paul Delafosse
            #### Bug Fixes
            - (**parser**) fix parser implementation - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - [@oknozor](https://github.com/oknozor)
            "
        );

        Ok(())
    }

    #[test]
    fn should_render_remote_template_monorepo_for_manual_bump() -> Result<()> {
        // Arrange
        let release = Release::fixture_manual();
        let mut renderer =
            Renderer::try_new(Template::Predefined(PredefinedTemplate::MONOERPO_REMOTE))?;
        renderer.add_context(RemoteContext::fixture());

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_doc_eq!(
            changelog,
            "## [1.0.0](https://github.com/cocogitto/cocogitto/compare/0.1.0..1.0.0) - 2015-09-05
            ### Packages
            - [one](crates/one) locked to [0.1.0](https://github.com/cocogitto/cocogitto/tree/0.1.0)
            - [two](crates/two) locked to [0.2.0](https://github.com/cocogitto/cocogitto/tree/0.2.0)
            ### Global changes
            #### Features
            - (**parser**) implement the changelog generator - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - [@oknozor](https://github.com/oknozor)
            - awesome feature - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - Paul Delafosse
            #### Bug Fixes
            - (**parser**) fix parser implementation - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - [@oknozor](https://github.com/oknozor)
            "
        );

        Ok(())
    }

    #[test]
    fn should_render_template_package() -> Result<()> {
        // Arrange
        let release = Release::fixture(Target::Package { name: "one" });
        let renderer =
            Renderer::try_new(Template::Predefined(PredefinedTemplate::PACKAGE_DEFAULT))?;

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_doc_eq!(
            changelog,
            "## 1.0.0 - 2015-09-05
            #### Features
            - (**parser**) implement the changelog generator - (17f7e23) - *oknozor*
            - awesome feature - (17f7e23) - Paul Delafosse
            #### Bug Fixes
            - (**parser**) fix parser implementation - (17f7e23) - *oknozor*
            "
        );

        Ok(())
    }

    #[test]
    fn should_render_full_hash_template_package() -> Result<()> {
        // Arrange
        let release = Release::fixture(Target::Package { name: "one" });
        let renderer =
            Renderer::try_new(Template::Predefined(PredefinedTemplate::PACKAGE_FULL_HASH))?;

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_doc_eq!(
            changelog,
            "#### Features
            - 17f7e23081db15e9318aeb37529b1d473cf41cbe - (**parser**) implement the changelog generator - @oknozor
            - 17f7e23081db15e9318aeb37529b1d473cf41cbe - awesome feature - Paul Delafosse
            #### Bug Fixes
            - 17f7e23081db15e9318aeb37529b1d473cf41cbe - (**parser**) fix parser implementation - @oknozor
            "
        );

        Ok(())
    }

    #[test]
    fn should_render_remote_template_package() -> Result<()> {
        // Arrange
        let release = Release::fixture(Target::Package { name: "one" });
        let mut renderer =
            Renderer::try_new(Template::Predefined(PredefinedTemplate::PACKAGE_REMOTE))?;
        renderer.add_context(RemoteContext::fixture());

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_doc_eq!(
            changelog,
            "## [1.0.0](https://github.com/cocogitto/cocogitto/compare/0.1.0..1.0.0) - 2015-09-05
            #### Features
            - (**parser**) implement the changelog generator - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - [@oknozor](https://github.com/oknozor)
            - awesome feature - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - Paul Delafosse
            #### Bug Fixes
            - (**parser**) fix parser implementation - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - [@oknozor](https://github.com/oknozor)
            "
        );

        Ok(())
    }

    #[test]
    fn should_render_unified_default() -> Result<()> {
        // Arrange
        let release = Release::fixture_monorepo();

        let renderer =
            Renderer::try_new(Template::Predefined(PredefinedTemplate::UNIFIED_DEFAULT))?;

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_doc_eq!(
            changelog,
            "## 1.0.0 - 2015-09-05
            ### Package updates
            - one bumped to 0.1.0
            - two bumped to 0.2.0
            ### All changes
            #### Features
            - (**parser**) implement the changelog generator - (17f7e23) - *oknozor*
            - awesome feature - (17f7e23) - Paul Delafosse
            #### Bug Fixes
            - (**parser**) fix parser implementation - (17f7e23) - *oknozor*
            "
        );

        Ok(())
    }

    #[test]
    fn should_render_unified_full_hash() -> Result<()> {
        // Arrange
        let release = Release::fixture_monorepo();

        let renderer =
            Renderer::try_new(Template::Predefined(PredefinedTemplate::UNIFIED_FULL_HASH))?;

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_doc_eq!(changelog,
            "### Package updates
            - one bumped to 0.1.0
            - two bumped to 0.2.0
            ### All changes
            #### Features
            - 17f7e23081db15e9318aeb37529b1d473cf41cbe - (**parser**) implement the changelog generator - @oknozor
            - 17f7e23081db15e9318aeb37529b1d473cf41cbe - awesome feature - Paul Delafosse
            #### Bug Fixes
            - 17f7e23081db15e9318aeb37529b1d473cf41cbe - (**parser**) fix parser implementation - @oknozor
            "
        );

        Ok(())
    }

    #[test]
    fn should_render_unified_remote() -> Result<()> {
        // Arrange
        let release = Release::fixture_monorepo();

        let mut renderer =
            Renderer::try_new(Template::Predefined(PredefinedTemplate::UNIFIED_REMOTE))?;
        renderer.add_context(RemoteContext::fixture());

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_doc_eq!(changelog,
            "## [1.0.0](https://github.com/cocogitto/cocogitto/compare/0.1.0..1.0.0) - 2015-09-05
            ### Package updates
            - [one](crates/one) bumped to [0.1.0](https://github.com/cocogitto/cocogitto/compare/0.2.0..0.1.0)
            - [two](crates/two) bumped to [0.2.0](https://github.com/cocogitto/cocogitto/compare/0.3.0..0.2.0)
            ### All changes
            #### Features
            - (**parser**) implement the changelog generator - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - [@oknozor](https://github.com/oknozor)
            - awesome feature - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - Paul Delafosse
            #### Bug Fixes
            - (**parser**) fix parser implementation - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - [@oknozor](https://github.com/oknozor)
            "
        );

        Ok(())
    }

    impl Release<'_> {
        pub fn fixture(target: Target) -> Release<'static> {
            let date =
                NaiveDateTime::parse_from_str("2015-09-05 23:56:04", "%Y-%m-%d %H:%M:%S").unwrap();

            let paul_delafosse = "Paul Delafosse";
            let a_commit_hash = "17f7e23081db15e9318aeb37529b1d473cf41cbe";
            let version = Tag::from_str(
                "1.0.0",
                Some(Oid::from_str("9bb5facac5724bc81385fdd740fedbb49056da00").unwrap()),
            )
            .unwrap();
            let from = Tag::from_str(
                "0.1.0",
                Some(Oid::from_str("fae3a288a1bc69b14f85a1d5fe57cee1964acd60").unwrap()),
            )
            .unwrap();
            Release {
                version: version.into(),
                from: from.into(),
                date,
                commits: vec![
                    ChangelogCommit {
                        author_username: Some("oknozor"),
                        commit: Commit {
                            oid: a_commit_hash.to_string(),
                            conventional: ConventionalCommit {
                                commit_type: CommitType::BugFix,
                                scope: Some("parser".to_string()),
                                summary: "fix parser implementation".to_string(),
                                body: Some("the body".to_string()),
                                footers: vec![Footer {
                                    token: "token".to_string(),
                                    content: "content".to_string(),
                                    ..Default::default()
                                }],
                                is_breaking_change: false,
                            },
                            author: paul_delafosse.to_string(),
                            date,
                        },
                    },
                    ChangelogCommit {
                        author_username: None,
                        commit: Commit {
                            oid: a_commit_hash.to_string(),
                            conventional: ConventionalCommit {
                                commit_type: CommitType::Feature,
                                scope: None,
                                summary: "awesome feature".to_string(),
                                body: Some("the body".to_string()),
                                footers: vec![Footer {
                                    token: "token".to_string(),
                                    content: "content".to_string(),
                                    ..Default::default()
                                }],
                                is_breaking_change: false,
                            },
                            author: paul_delafosse.to_string(),
                            date,
                        },
                    },
                    ChangelogCommit {
                        author_username: Some("oknozor"),
                        commit: Commit {
                            oid: a_commit_hash.to_string(),
                            conventional: ConventionalCommit {
                                commit_type: CommitType::Feature,
                                scope: Some("parser".to_string()),
                                summary: "implement the changelog generator".to_string(),
                                body: Some("the body".to_string()),
                                footers: vec![Footer {
                                    token: "token".to_string(),
                                    content: "content".to_string(),
                                    ..Default::default()
                                }],
                                is_breaking_change: false,
                            },
                            author: "James Delleck".to_string(),
                            date,
                        },
                    },
                ],
                packages: Vec::new(),
                target,
            }
        }

        fn fixture_monorepo() -> Release<'static> {
            let mut release = Self::fixture(Target::Monorepo { manual: false });
            release.packages = vec![
                PackageBump {
                    package_name: "one",
                    package_path: Path::new("crates/one"),
                    version: Tag::from_str(
                        "0.1.0",
                        Some(Oid::from_str("fae3a288a1bc69b14f85a1d5fe57cee1964acd60").unwrap()),
                    )
                    .unwrap()
                    .into(),
                    from: Some(
                        Tag::from_str(
                            "0.2.0",
                            Some(
                                Oid::from_str("fae3a288a1bc69b14f85a1d5fe57cee1964acd60").unwrap(),
                            ),
                        )
                        .unwrap()
                        .into(),
                    ),
                },
                PackageBump {
                    package_name: "two",
                    package_path: Path::new("crates/two"),
                    version: Tag::from_str(
                        "0.2.0",
                        Some(Oid::from_str("fae3a288a1bc69b14f85a1d5fe57cee1964acd60").unwrap()),
                    )
                    .unwrap()
                    .into(),
                    from: Some(
                        Tag::from_str(
                            "0.3.0",
                            Some(
                                Oid::from_str("fae3a288a1bc69b14f85a1d5fe57cee1964acd60").unwrap(),
                            ),
                        )
                        .unwrap()
                        .into(),
                    ),
                },
            ];
            release
        }

        fn fixture_manual() -> Self {
            let mut release = Self::fixture(Target::Monorepo { manual: true });
            release.packages = vec![
                PackageBump {
                    package_name: "one",
                    package_path: Path::new("crates/one"),
                    version: Tag::from_str(
                        "0.1.0",
                        Some(Oid::from_str("fae3a288a1bc69b14f85a1d5fe57cee1964acd60").unwrap()),
                    )
                    .unwrap()
                    .into(),
                    from: None,
                },
                PackageBump {
                    package_name: "two",
                    package_path: Path::new("crates/two"),
                    version: Tag::from_str(
                        "0.2.0",
                        Some(Oid::from_str("fae3a288a1bc69b14f85a1d5fe57cee1964acd60").unwrap()),
                    )
                    .unwrap()
                    .into(),
                    from: None,
                },
            ];
            release
        }
    }

    impl RemoteContext {
        fn fixture() -> Self {
            RemoteContext::try_new(
                Some("github.com".into()),
                Some("cocogitto".into()),
                Some("cocogitto".into()),
            )
            .unwrap()
        }
    }
}
