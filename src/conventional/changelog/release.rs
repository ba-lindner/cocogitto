use std::fmt::Display;

use chrono::{NaiveDateTime, Utc};
use conventional_commit_parser::commit::Footer;
use git2::Oid;
use serde::{Serialize, Serializer};

use crate::conventional::changelog::template::PackageBumpContext;
use crate::conventional::commit::Commit;
use crate::git::rev::cache::CommitInfo;
use crate::git::rev::filters::CommitRangeFilter;
use crate::git::rev::CommitRange;
use crate::git::tag::Tag;
use crate::settings;
use colored::Colorize;

use log::warn;

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct ReleaseVersion {
    #[serde(serialize_with = "serialize_oid")]
    pub id: Oid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<Tag>,
}

fn serialize_oid<S: Serializer>(oid: &Oid, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&oid.to_string())
}

impl ReleaseVersion {
    fn empty() -> Self {
        Self {
            id: Oid::zero(),
            tag: None,
        }
    }
}

impl CommitInfo {
    pub fn into_release_version(self, filter: CommitRangeFilter<'_>) -> ReleaseVersion {
        ReleaseVersion {
            id: self.oid,
            tag: self.tags.into_iter().find(|tag| filter.matches_tag(tag)),
        }
    }
}

impl From<Oid> for ReleaseVersion {
    fn from(value: Oid) -> Self {
        Self {
            id: value,
            tag: None,
        }
    }
}

impl From<Tag> for ReleaseVersion {
    fn from(value: Tag) -> Self {
        Self {
            id: value.oid.unwrap_or(Oid::zero()),
            tag: Some(value),
        }
    }
}

impl Display for ReleaseVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(tag) = &self.tag {
            tag.fmt(f)
        } else {
            self.id.fmt(f)
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Release<'a> {
    pub version: ReleaseVersion,
    pub from: ReleaseVersion,
    pub date: NaiveDateTime,
    pub commits: Vec<ChangelogCommit>,
    pub packages: Vec<PackageBumpContext<'a>>,
}

impl Release<'_> {
    pub fn empty() -> Self {
        Self {
            version: ReleaseVersion::empty(),
            from: ReleaseVersion::empty(),
            date: Utc::now().naive_local(),
            commits: Vec::new(),
            packages: Vec::new(),
        }
    }

    pub fn build(range: CommitRange<'_>, filter: CommitRangeFilter<'_>) -> Vec<Self> {
        range
            .split_at_tags(filter)
            .into_iter()
            .map(|range| Self::build_single(range, filter))
            .collect()
    }

    pub fn build_single(range: CommitRange<'_>, filter: CommitRangeFilter<'_>) -> Self {
        Self {
            version: range.to().into_release_version(filter),
            from: range
                .prev()
                .unwrap_or(range.from())
                .into_release_version(filter),
            date: Utc::now().naive_local(),
            commits: range
                .into_iter()
                .filter_map(|commit| {
                    Commit::from_git_commit(&commit)
                        .inspect_err(|err| warn!("{}", err.to_string().red()))
                        .ok()
                })
                .filter(|commit| !commit.should_omit())
                .map(ChangelogCommit::from)
                .collect(),
            packages: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct ChangelogCommit {
    pub author_username: Option<&'static str>,
    pub commit: Commit,
}

impl From<Commit> for ChangelogCommit {
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
    use anyhow::Result;
    use chrono::NaiveDateTime;
    use conventional_commit_parser::commit::{CommitType, ConventionalCommit, Footer};
    use git2::Oid;
    use indoc::indoc;
    use pretty_assertions::assert_eq;
    use speculoos::prelude::*;

    use crate::conventional::changelog::release::{ChangelogCommit, Release, ReleaseVersion};
    use crate::conventional::changelog::renderer::Renderer;
    use crate::conventional::changelog::template::{
        MonoRepoContext, PackageBumpContext, PackageContext, RemoteContext, Template,
    };
    use crate::conventional::commit::Commit;
    use crate::git::repository::Repository;

    use crate::git::rev::filters::CommitRangeFilter;
    use crate::git::tag::Tag;

    #[test]
    fn should_get_a_release() -> anyhow::Result<()> {
        let repo = Repository::open(".")?;
        let iter = repo.revwalk_pattern("..")?;
        let release = Release::build(iter, CommitRangeFilter::None);
        assert_that!(release).is_not_empty();
        assert_that!(release[0].commits).is_not_empty();
        Ok(())
    }

    #[test]
    fn should_render_default_template() -> Result<()> {
        // Arrange
        let release = Release::fixture();
        let renderer = Renderer::default();

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_eq!(
            changelog.0,
            indoc! {
                "## 1.0.0 - 2015-09-05
                #### Bug Fixes
                - **(parser)** fix parser implementation - (17f7e23) - *oknozor*
                #### Features
                - **(parser)** implement the changelog generator - (17f7e23) - *oknozor*
                - awesome feature - (17f7e23) - Paul Delafosse
                "
            }
        );

        Ok(())
    }

    #[test]
    fn should_render_full_hash_template() -> Result<()> {
        // Arrange
        let release = Release::fixture();
        let renderer = Renderer::new(Template::FULL_HASH)?;

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_eq!(
            changelog.0,
            indoc! {
                "#### Bug Fixes
                - 17f7e23081db15e9318aeb37529b1d473cf41cbe - **(parser)** fix parser implementation - @oknozor
                #### Features
                - 17f7e23081db15e9318aeb37529b1d473cf41cbe - **(parser)** implement the changelog generator - @oknozor
                - 17f7e23081db15e9318aeb37529b1d473cf41cbe - awesome feature - Paul Delafosse

                "
            }
        );

        Ok(())
    }

    #[test]
    fn should_render_github_template() -> Result<()> {
        // Arrange
        let release = Release::fixture();
        let renderer = Renderer::new(Template::REMOTE)?.with_context(RemoteContext::fixture());

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_eq!(
            changelog.0,
            indoc! {
                "## [1.0.0](https://github.com/cocogitto/cocogitto/compare/0.1.0..1.0.0) - 2015-09-05
                #### Bug Fixes
                - **(parser)** fix parser implementation - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - [@oknozor](https://github.com/oknozor)
                #### Features
                - **(parser)** implement the changelog generator - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - [@oknozor](https://github.com/oknozor)
                - awesome feature - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - Paul Delafosse
                "
            }
        );

        Ok(())
    }

    #[test]
    fn should_render_template_monorepo() -> Result<()> {
        // Arrange
        let release = Release::fixture_monorepo();
        let renderer = Renderer::new(Template::MONOREPO_DEFAULT)?;

        let renderer = monorepo_renderer(renderer, false)?;

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_eq!(
            changelog.0,
            indoc! {
                "## 1.0.0 - 2015-09-05
                ### Package updates
                - one bumped to 0.1.0
                - two bumped to 0.2.0
                ### Global changes
                #### Bug Fixes
                - **(parser)** fix parser implementation - (17f7e23) - *oknozor*
                #### Features
                - **(parser)** implement the changelog generator - (17f7e23) - *oknozor*
                - awesome feature - (17f7e23) - Paul Delafosse
                "
            }
        );

        Ok(())
    }

    #[test]
    fn should_render_template_monorepo_for_manual_bump() -> Result<()> {
        // Arrange
        let release = Release::fixture_manual();
        let renderer = Renderer::new(Template::MONOREPO_DEFAULT)?;

        let renderer = monorepo_renderer(renderer, true)?;

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_eq!(
            changelog.0,
            indoc! {
                "## 1.0.0 - 2015-09-05
                ### Packages
                - one locked to 0.1.0
                - two locked to 0.2.0
                ### Global changes
                #### Bug Fixes
                - **(parser)** fix parser implementation - (17f7e23) - *oknozor*
                #### Features
                - **(parser)** implement the changelog generator - (17f7e23) - *oknozor*
                - awesome feature - (17f7e23) - Paul Delafosse
                "
            }
        );

        Ok(())
    }

    #[test]
    fn should_render_full_hash_template_monorepo() -> Result<()> {
        // Arrange
        let release = Release::fixture_monorepo();
        let renderer = Renderer::new(Template::MONOREPO_FULL_HASH)?;

        let renderer = monorepo_renderer(renderer, false)?;

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_eq!(
            changelog.0,
            indoc! {
                "### Package updates
                - one bumped to 0.1.0
                - two bumped to 0.2.0
                ### Global changes
                #### Bug Fixes
                - 17f7e23081db15e9318aeb37529b1d473cf41cbe - **(parser)** fix parser implementation - @oknozor
                #### Features
                - 17f7e23081db15e9318aeb37529b1d473cf41cbe - **(parser)** implement the changelog generator - @oknozor
                - 17f7e23081db15e9318aeb37529b1d473cf41cbe - awesome feature - Paul Delafosse

                "
            }
        );

        Ok(())
    }

    #[test]
    fn should_render_full_hash_template_manual_monorepo() -> Result<()> {
        // Arrange
        let release = Release::fixture_manual();
        let renderer = Renderer::new(Template::MONOREPO_FULL_HASH)?;

        let renderer = monorepo_renderer(renderer, true)?;

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_eq!(
            changelog.0,
            indoc! {
                "### Packages
                - one locked to 0.1.0
                - two locked to 0.2.0
                ### Global changes
                #### Bug Fixes
                - 17f7e23081db15e9318aeb37529b1d473cf41cbe - **(parser)** fix parser implementation - @oknozor
                #### Features
                - 17f7e23081db15e9318aeb37529b1d473cf41cbe - **(parser)** implement the changelog generator - @oknozor
                - 17f7e23081db15e9318aeb37529b1d473cf41cbe - awesome feature - Paul Delafosse

                "
            }
        );

        Ok(())
    }

    #[test]
    fn should_render_remote_template_monorepo() -> Result<()> {
        // Arrange
        let release = Release::fixture_monorepo();
        let renderer =
            Renderer::new(Template::MONOREPO_REMOTE)?.with_context(RemoteContext::fixture());

        let renderer = monorepo_renderer(renderer, false)?;

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_eq!(
            changelog.0,
            indoc! {
                "## [1.0.0](https://github.com/cocogitto/cocogitto/compare/0.1.0..1.0.0) - 2015-09-05
                ### Package updates
                - [0.1.0](crates/one) bumped to [0.1.0](https://github.com/cocogitto/cocogitto/compare/0.2.0..0.1.0)
                - [0.2.0](crates/two) bumped to [0.2.0](https://github.com/cocogitto/cocogitto/compare/0.3.0..0.2.0)
                ### Global changes
                #### Bug Fixes
                - **(parser)** fix parser implementation - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - [@oknozor](https://github.com/oknozor)
                #### Features
                - **(parser)** implement the changelog generator - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - [@oknozor](https://github.com/oknozor)
                - awesome feature - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - Paul Delafosse
                "
            }
        );

        Ok(())
    }

    #[test]
    fn should_render_template_package() -> Result<()> {
        // Arrange
        let release = Release::fixture();
        let renderer = Renderer::new(Template::PACKAGE_DEFAULT)?;

        let renderer = package_renderer(renderer)?;

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_eq!(
            changelog.0,
            indoc! {
                "## 1.0.0 - 2015-09-05
                #### Bug Fixes
                - **(parser)** fix parser implementation - (17f7e23) - *oknozor*
                #### Features
                - **(parser)** implement the changelog generator - (17f7e23) - *oknozor*
                - awesome feature - (17f7e23) - Paul Delafosse
                "
            }
        );

        Ok(())
    }

    #[test]
    fn should_render_full_hash_template_package() -> Result<()> {
        // Arrange
        let release = Release::fixture();
        let renderer = Renderer::new(Template::PACKAGE_FULL_HASH)?;

        let renderer = package_renderer(renderer)?;

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_eq!(
            changelog.0,
            indoc! {
                "#### Bug Fixes
                - 17f7e23081db15e9318aeb37529b1d473cf41cbe - **(parser)** fix parser implementation - @oknozor
                #### Features
                - 17f7e23081db15e9318aeb37529b1d473cf41cbe - **(parser)** implement the changelog generator - @oknozor
                - 17f7e23081db15e9318aeb37529b1d473cf41cbe - awesome feature - Paul Delafosse

                "
            }
        );

        Ok(())
    }

    #[test]
    fn should_render_remote_template_package() -> Result<()> {
        // Arrange
        let release = Release::fixture();
        let renderer =
            Renderer::new(Template::PACKAGE_REMOTE)?.with_context(RemoteContext::fixture());

        let renderer = package_renderer(renderer)?;

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_eq!(
            changelog.0,
            indoc! {
                "## [1.0.0](https://github.com/cocogitto/cocogitto/compare/0.1.0..1.0.0) - 2015-09-05
                #### Bug Fixes
                - **(parser)** fix parser implementation - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - [@oknozor](https://github.com/oknozor)
                #### Features
                - **(parser)** implement the changelog generator - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - [@oknozor](https://github.com/oknozor)
                - awesome feature - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - Paul Delafosse
                "
            }
        );

        Ok(())
    }

    #[test]
    fn should_render_remote_template_monorepo_for_manual_bump() -> Result<()> {
        // Arrange
        let release = Release::fixture_manual();
        let renderer =
            Renderer::new(Template::MONOREPO_REMOTE)?.with_context(RemoteContext::fixture());

        let renderer = monorepo_renderer(renderer, true)?;

        // Act
        let changelog = renderer.render_single(&release)?;

        // Assert
        assert_eq!(
            changelog.0,
            indoc! {
                "## [1.0.0](https://github.com/cocogitto/cocogitto/compare/0.1.0..1.0.0) - 2015-09-05
                ### Packages
                - [0.1.0](crates/one) locked to [0.1.0](https://github.com/cocogitto/cocogitto/tree/0.1.0)
                - [0.2.0](crates/two) locked to [0.2.0](https://github.com/cocogitto/cocogitto/tree/0.2.0)
                ### Global changes
                #### Bug Fixes
                - **(parser)** fix parser implementation - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - [@oknozor](https://github.com/oknozor)
                #### Features
                - **(parser)** implement the changelog generator - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - [@oknozor](https://github.com/oknozor)
                - awesome feature - ([17f7e23](https://github.com/cocogitto/cocogitto/commit/17f7e23081db15e9318aeb37529b1d473cf41cbe)) - Paul Delafosse
                "
            }
        );

        Ok(())
    }

    fn monorepo_renderer(renderer: Renderer, package_lock: bool) -> Result<Renderer> {
        let renderer = renderer.with_context(MonoRepoContext { package_lock });

        Ok(renderer)
    }

    fn package_renderer(renderer: Renderer) -> Result<Renderer> {
        let renderer = renderer.with_context(PackageContext {
            package_name: "one",
        });

        Ok(renderer)
    }

    impl RemoteContext {
        fn fixture() -> Self {
            RemoteContext::try_new(
                Some("github.com".into()),
                Some("cocogitto".into()),
                Some("cocogitto".into()),
            )
            .expect("all fields set")
        }
    }

    impl Release<'_> {
        pub fn fixture() -> Release<'static> {
            let date =
                NaiveDateTime::parse_from_str("2015-09-05 23:56:04", "%Y-%m-%d %H:%M:%S").unwrap();

            let paul_delafosse = "Paul Delafosse";
            let a_commit_hash = "17f7e23081db15e9318aeb37529b1d473cf41cbe";
            let vers_oid = Oid::from_str("9bb5facac5724bc81385fdd740fedbb49056da00").unwrap();
            let version = Tag::from_str("1.0.0", Some(vers_oid)).unwrap();
            let from_oid = Oid::from_str("fae3a288a1bc69b14f85a1d5fe57cee1964acd60").unwrap();
            let from = Tag::from_str("0.1.0", Some(from_oid)).unwrap();
            Release {
                version: ReleaseVersion {
                    id: vers_oid,
                    tag: Some(version),
                },
                from: ReleaseVersion {
                    id: from_oid,
                    tag: Some(from),
                },
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
            }
        }

        pub fn fixture_monorepo() -> Release<'static> {
            let mut release = Self::fixture();
            release.packages = vec![
                PackageBumpContext {
                    package_name: "one",
                    package_path: "crates/one".to_string(),
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
                PackageBumpContext {
                    package_name: "two",
                    package_path: "crates/two".to_string(),
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

        pub fn fixture_manual() -> Release<'static> {
            let mut release = Self::fixture();
            release.packages = vec![
                PackageBumpContext {
                    package_name: "one",
                    package_path: "crates/one".to_string(),
                    version: Tag::from_str(
                        "0.1.0",
                        Some(Oid::from_str("fae3a288a1bc69b14f85a1d5fe57cee1964acd60").unwrap()),
                    )
                    .unwrap()
                    .into(),
                    from: None,
                },
                PackageBumpContext {
                    package_name: "two",
                    package_path: "crates/two".to_string(),
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
}
