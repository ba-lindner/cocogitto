use chrono::{NaiveDateTime, Utc};
use conventional_commit_parser::commit::{Footer, Separator};
use serde::Serialize;

use crate::conventional::changelog::context::PackageBumpContext;
use crate::conventional::commit::Commit;
use crate::git::oid::ReleaseVersion;
use crate::git::repository::Repository;
use crate::git::rev::revspec::RevSpecPattern2;
use crate::git::rev::CommitIter;
use crate::git::tag::Tag;
use crate::{settings, SETTINGS};
use colored::Colorize;

use crate::conventional::changelog::error::ChangelogError;
use log::warn;

#[derive(Debug, Serialize)]
pub struct Release {
    pub version: ReleaseVersion,
    pub from: ReleaseVersion,
    pub date: NaiveDateTime,
    pub commits: Vec<ChangelogCommit>,
    pub previous: Option<Box<Release>>,
    pub packages: Vec<PackageBumpContext<'static>>,
}

impl TryFrom<CommitIter<'_>> for Release {
    type Error = ChangelogError;

    fn try_from(commits: CommitIter<'_>) -> Result<Self, Self::Error> {
        let releases = commits.split_at_tags();

        let mut current = None;

        for release in releases {
            let (from, version) = release.version_range();
            let date = release
                .iter_commits()
                .next()
                .and_then(|commit| chrono::DateTime::from_timestamp(commit.time().seconds(), 0))
                .unwrap_or_else(Utc::now)
                .naive_utc();
            let next = Release {
                version,
                from,
                date,
                commits: release
                    .ignore_commits_by_settings()
                    .iter_commits()
                    .filter(|commit| commit.message().is_some())
                    .filter_map(|commit| {
                        Commit::from_git_commit(commit)
                            .inspect_err(|err| warn!("{}", err.to_string().red()))
                            .ok()
                    })
                    .filter(|commit| !commit.should_omit())
                    .map(ChangelogCommit::from)
                    .collect(),
                previous: current.map(Box::new),
                packages: Vec::new(),
            };

            current = Some(next);
        }

        current.ok_or(ChangelogError::EmptyRelease)
    }
}

impl Release {
    pub(crate) fn add_package_bumps(&mut self, repository: &Repository) -> anyhow::Result<()> {
        let oid = |version: &ReleaseVersion| Some(version.id).filter(|oid| !oid.is_zero());
        let commits = repository.revwalk(RevSpecPattern2 {
            from: oid(&self.from),
            to: oid(&self.version),
        })?;
        for (name, package) in SETTINGS.list_packages() {
            if !package.public_api {
                continue;
            }
            let mut tags = None::<(&Tag, &Tag)>;
            for (info, _) in &commits {
                let tag = info
                    .tags
                    .iter()
                    .find(|tag| tag.package.as_ref() == Some(name));
                let (new, old) = tags.unzip();
                tags = new.or(tag).zip(tag.or(old));
            }
            let Some((new, old)) = tags else {
                continue;
            };
            let mut from = repository.get_previous_tag(old).unwrap_or_default();
            from.package = Some(name.clone());
            self.packages.push(PackageBumpContext {
                package_name: name,
                package_path: package.path.to_string_lossy().to_string(),
                version: new.clone().into(),
                from: Some(from.into()),
            });
        }
        if let Some(previous) = &mut self.previous {
            previous.add_package_bumps(repository)?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct ChangelogCommit {
    pub author_username: Option<String>,
    pub commit: Commit,
}

impl From<Commit> for ChangelogCommit {
    fn from(commit: Commit) -> Self {
        let author_username =
            settings::commit_username(&commit.author).map(|username| username.to_string());

        ChangelogCommit {
            author_username,
            commit,
        }
    }
}

/// Either a simple conventional commit footer (ex: Myfooter: value, Other #value)
/// or GitHub specific trailers:
/// Co-authored-by: Paul Delafosse <paul.delafosse@protonmail.com>
/// Closes #123
#[derive(Serialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ChangelogFooter<'a> {
    GithubCoAuthoredBy {
        user: &'a str,
        username: Option<&'a str>,
    },
    GithubCloses {
        gh_reference: &'a str,
    },
    Footer {
        token: &'a str,
        content: &'a str,
    },
}

impl<'a> From<&'a Footer> for ChangelogFooter<'a> {
    fn from(footer: &'a Footer) -> Self {
        match footer.token.as_str().to_lowercase().as_str() {
            "co-authored-by" if footer.token_separator == Separator::Colon => {
                let user = footer
                    .content
                    .split('<')
                    .next()
                    .map(str::trim)
                    .unwrap_or(footer.content.as_str());

                let username = settings::commit_username(user);

                Self::GithubCoAuthoredBy { user, username }
            }
            "close" | "closes" | "closed" | "fix" | "fixes" | "fixed" | "resolve" | "resolves"
            | "resolved"
                if footer.token_separator == Separator::Hash =>
            {
                Self::GithubCloses {
                    gh_reference: footer.content.as_str(),
                }
            }
            _ => Self::Footer {
                token: footer.token.as_str(),
                content: footer.content.as_str(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use conventional_commit_parser::commit::{Footer, Separator};
    use speculoos::prelude::*;

    #[test]
    fn changelog_footer_from_github_co_authored_by() {
        // Arrange
        let footer = Footer {
            token: "Co-authored-by".to_string(),
            token_separator: Separator::Colon,
            content: "Paul Delafosse <paul.delafosse@protonmail.com>".to_string(),
        };

        // Act
        let changelog_footer = ChangelogFooter::from(&footer);

        // Assert - since there's no author mapping in the test environment,
        // the username should be None
        assert_that!(changelog_footer).matches(|ch| {
            matches!(
                ch,
                ChangelogFooter::GithubCoAuthoredBy {
                    user: "Paul Delafosse",
                    username: Some("oknozor")
                }
            )
        });
    }

    #[test]
    fn changelog_footer_from_github_closes() {
        // Arrange
        let footer = Footer {
            token: "Closes".to_string(),
            token_separator: Separator::Hash,
            content: "123".to_string(),
        };

        // Act
        let changelog_footer = ChangelogFooter::from(&footer);

        // Assert
        assert_that!(changelog_footer).matches(|ch| {
            matches!(
                ch,
                ChangelogFooter::GithubCloses {
                    gh_reference: "123"
                }
            )
        });
    }

    #[test]
    fn changelog_footer_from_generic_footer() {
        // Arrange
        let footer = Footer {
            token: "MyFooter".to_string(),
            token_separator: Separator::Colon,
            content: "Some value".to_string(),
        };

        // Act
        let changelog_footer = ChangelogFooter::from(&footer);

        // Assert
        assert_that!(changelog_footer).matches(|ch| {
            matches!(
                ch,
                ChangelogFooter::Footer {
                    token: "MyFooter",
                    content: "Some value"
                }
            )
        });
    }
}
