use chrono::{NaiveDateTime, Utc};
use conventional_commit_parser::commit::Footer;
use serde::Serialize;

use crate::conventional::commit::Commit;
use crate::git::rev::cache::CommitInfo;
use crate::git::rev::CommitIter;
use crate::settings;
use colored::Colorize;

use crate::conventional::changelog::error::ChangelogError;
use log::warn;

#[derive(Debug, Serialize)]
pub struct Release {
    pub version: CommitInfo,
    pub from: CommitInfo,
    pub date: NaiveDateTime,
    pub commits: Vec<ChangelogCommit>,
    pub previous: Option<Box<Release>>,
}

impl TryFrom<CommitIter<'_>> for Release {
    type Error = ChangelogError;

    fn try_from(mut commits: CommitIter<'_>) -> Result<Self, Self::Error> {
        commits.0.reverse();
        let from = commits
            .0
            .first()
            .ok_or(ChangelogError::EmptyRelease)?
            .1
            .clone();
        commits
            .0
            .split_inclusive(|(_, info)| !info.tags.is_empty())
            .filter_map(|commits| {
                let version = &commits.last()?.1;
                let commits = commits.iter().map(|(commit, _)| commit).rev();
                Some((version, commits))
            })
            .fold((None, from), |(prev, from), (version, commits)| {
                (
                    Some(Release {
                        version: version.clone(),
                        from,
                        date: Utc::now().naive_local(),
                        commits: commits
                            .filter_map(|commit| {
                                Commit::from_git_commit(commit)
                                    .inspect_err(|err| warn!("{}", err.to_string().red()))
                                    .ok()
                            })
                            .filter(|commit| !commit.should_omit())
                            .map(ChangelogCommit::from)
                            .collect(),
                        previous: prev.map(Box::new),
                    }),
                    version.clone(),
                )
            })
            .0
            .ok_or(ChangelogError::EmptyRelease)
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

    use crate::conventional::changelog::release::{ChangelogCommit, Release};
    use crate::conventional::changelog::renderer::Renderer;
    use crate::conventional::changelog::template::{
        MonoRepoContext, PackageBumpContext, PackageContext, RemoteContext, Template, TemplateKind,
    };
    use crate::conventional::commit::Commit;
    use crate::git::oid::OidOf;
    use crate::git::repository::Repository;

    use crate::git::rev::cache::CommitInfo;
    use crate::git::tag::Tag;

    #[test]
    fn should_get_a_release() -> anyhow::Result<()> {
        let repo = Repository::open(".")?;
        let iter = repo.revwalk_pattern("..")?;
        let release = Release::try_from(iter);
        assert_that!(release)
            .is_ok()
            .matches(|r| !r.commits.is_empty());
        Ok(())
    }

    #[test]
    fn should_render_default_template() -> Result<()> {
        // Arrange
        let release = Release::fixture();
        let mut renderer = Renderer::default();

        // Act
        let changelog = renderer.render(&release)?;

        // Assert
        assert_eq!(
            changelog,
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
        let mut renderer = Renderer::try_new(Template {
            remote_context: None,
            kind: TemplateKind::FullHash,
        })?;

        // Act
        let changelog = renderer.render(&release)?;

        // Assert
        assert_eq!(
            changelog,
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
        let mut renderer = Renderer::try_new(Template {
            remote_context: RemoteContext::try_new(
                Some("github.com".into()),
                Some("cocogitto".into()),
                Some("cocogitto".into()),
            ),
            kind: TemplateKind::Remote,
        })?;

        // Act
        let changelog = renderer.render(&release)?;

        // Assert
        assert_eq!(
            changelog,
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
        let release = Release::fixture();
        let renderer = Renderer::try_new(Template {
            remote_context: None,
            kind: TemplateKind::MonorepoDefault,
        })?;

        let mut renderer = monorepo_renderer(renderer)?;

        // Act
        let changelog = renderer.render(&release)?;

        // Assert
        assert_eq!(
            changelog,
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
        let release = Release::fixture();
        let renderer = Renderer::try_new(Template {
            remote_context: None,
            kind: TemplateKind::MonorepoDefault,
        })?;

        let mut renderer = monorepo_manual_bump_rendered(renderer)?;

        // Act
        let changelog = renderer.render(&release)?;

        // Assert
        assert_eq!(
            changelog,
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
        let release = Release::fixture();
        let renderer = Renderer::try_new(Template {
            remote_context: None,
            kind: TemplateKind::MonorepoFullHash,
        })?;

        let mut renderer = monorepo_renderer(renderer)?;

        // Act
        let changelog = renderer.render(&release)?;

        // Assert
        assert_eq!(
            changelog,
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
        let release = Release::fixture();
        let renderer = Renderer::try_new(Template {
            remote_context: None,
            kind: TemplateKind::MonorepoFullHash,
        })?;

        let mut renderer = monorepo_manual_bump_rendered(renderer)?;

        // Act
        let changelog = renderer.render(&release)?;

        // Assert
        assert_eq!(
            changelog,
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
        let release = Release::fixture();
        let renderer = Renderer::try_new(Template {
            remote_context: RemoteContext::try_new(
                Some("github.com".into()),
                Some("cocogitto".into()),
                Some("cocogitto".into()),
            ),
            kind: TemplateKind::MonorepoRemote,
        })?;

        let mut renderer = monorepo_renderer(renderer)?;

        // Act
        let changelog = renderer.render(&release)?;

        // Assert
        assert_eq!(
            changelog,
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
        let renderer = Renderer::try_new(Template {
            remote_context: None,
            kind: TemplateKind::PackageDefault,
        })?;

        let mut renderer = package_renderer(renderer)?;

        // Act
        let changelog = renderer.render(&release)?;

        // Assert
        assert_eq!(
            changelog,
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
        let renderer = Renderer::try_new(Template {
            remote_context: None,
            kind: TemplateKind::PackageFullHash,
        })?;

        let mut renderer = package_renderer(renderer)?;

        // Act
        let changelog = renderer.render(&release)?;

        // Assert
        assert_eq!(
            changelog,
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
        let renderer = Renderer::try_new(Template {
            remote_context: RemoteContext::try_new(
                Some("github.com".into()),
                Some("cocogitto".into()),
                Some("cocogitto".into()),
            ),
            kind: TemplateKind::PackageRemote,
        })?;

        let mut renderer = package_renderer(renderer)?;

        // Act
        let changelog = renderer.render(&release)?;

        // Assert
        assert_eq!(
            changelog,
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
        let release = Release::fixture();
        let renderer = Renderer::try_new(Template {
            remote_context: RemoteContext::try_new(
                Some("github.com".into()),
                Some("cocogitto".into()),
                Some("cocogitto".into()),
            ),
            kind: TemplateKind::MonorepoRemote,
        })?;

        let mut renderer = monorepo_manual_bump_rendered(renderer)?;

        // Act
        let changelog = renderer.render(&release)?;

        // Assert
        assert_eq!(
            changelog,
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

    fn monorepo_renderer(renderer: Renderer) -> Result<Renderer> {
        let renderer = renderer.with_monorepo_context(MonoRepoContext {
            package_lock: false,
            packages: vec![
                PackageBumpContext {
                    package_name: "one",
                    package_path: "crates/one".to_string(),
                    version: OidOf::Tag(Tag::from_str(
                        "0.1.0",
                        Some(Oid::from_str("fae3a288a1bc69b14f85a1d5fe57cee1964acd60").unwrap()),
                    )?),
                    from: Some(OidOf::Tag(Tag::from_str(
                        "0.2.0",
                        Some(Oid::from_str("fae3a288a1bc69b14f85a1d5fe57cee1964acd60").unwrap()),
                    )?)),
                },
                PackageBumpContext {
                    package_name: "two",
                    package_path: "crates/two".to_string(),
                    version: OidOf::Tag(Tag::from_str(
                        "0.2.0",
                        Some(Oid::from_str("fae3a288a1bc69b14f85a1d5fe57cee1964acd60").unwrap()),
                    )?),
                    from: Some(OidOf::Tag(Tag::from_str(
                        "0.3.0",
                        Some(Oid::from_str("fae3a288a1bc69b14f85a1d5fe57cee1964acd60").unwrap()),
                    )?)),
                },
            ],
        });

        Ok(renderer)
    }

    fn monorepo_manual_bump_rendered(renderer: Renderer) -> Result<Renderer> {
        let renderer = renderer.with_monorepo_context(MonoRepoContext {
            package_lock: true,
            packages: vec![
                PackageBumpContext {
                    package_name: "one",
                    package_path: "crates/one".to_string(),
                    version: OidOf::Tag(Tag::from_str(
                        "0.1.0",
                        Some(Oid::from_str("fae3a288a1bc69b14f85a1d5fe57cee1964acd60").unwrap()),
                    )?),
                    from: None,
                },
                PackageBumpContext {
                    package_name: "two",
                    package_path: "crates/two".to_string(),
                    version: OidOf::Tag(Tag::from_str(
                        "0.2.0",
                        Some(Oid::from_str("fae3a288a1bc69b14f85a1d5fe57cee1964acd60").unwrap()),
                    )?),
                    from: None,
                },
            ],
        });

        Ok(renderer)
    }

    fn package_renderer(renderer: Renderer) -> Result<Renderer> {
        let renderer = renderer.with_package_context(PackageContext {
            package_name: "one",
        });

        Ok(renderer)
    }

    impl Release {
        pub fn fixture() -> Release {
            let date =
                NaiveDateTime::parse_from_str("2015-09-05 23:56:04", "%Y-%m-%d %H:%M:%S").unwrap();

            let paul_delafosse = "Paul Delafosse";
            let a_commit_hash = "17f7e23081db15e9318aeb37529b1d473cf41cbe";
            let vers_oid = Oid::from_str("9bb5facac5724bc81385fdd740fedbb49056da00").unwrap();
            let version = Tag::from_str("1.0.0", Some(vers_oid)).unwrap();
            let from_oid = Oid::from_str("fae3a288a1bc69b14f85a1d5fe57cee1964acd60").unwrap();
            let from = Tag::from_str("0.1.0", Some(from_oid)).unwrap();
            Release {
                version: CommitInfo {
                    is_head: false,
                    is_first: false,
                    oid: vers_oid,
                    tags: vec![version],
                },
                from: CommitInfo {
                    is_head: false,
                    is_first: false,
                    oid: from_oid,
                    tags: vec![from],
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
                previous: None,
            }
        }
    }
}
