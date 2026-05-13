use chrono::{NaiveDateTime, Utc};
use conventional_commit_parser::commit::{Footer, Separator};
use serde::Serialize;

use crate::conventional::commit::Commit;
use crate::git::oid::ReleaseVersion;
use crate::git::rev::CommitIter;
use crate::settings;
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
}

impl TryFrom<CommitIter<'_>> for Release {
    type Error = ChangelogError;

    fn try_from(commits: CommitIter<'_>) -> Result<Self, Self::Error> {
        let releases = commits.split_at_tags();

        let mut current = None;

        for release in releases {
            let (from, version) = release.version_range();
            let date = chrono::DateTime::from_timestamp(
                release.iter_commits().next().unwrap().time().seconds(),
                0,
            )
            .unwrap_or_else(Utc::now)
            .naive_utc();
            let next = Release {
                version,
                from,
                date,
                commits: release
                    .ignore_commits_by_settings()
                    .into_iter()
                    .filter(|(_commit, commit)| commit.message().is_some())
                    .filter_map(|(_, commit)| match Commit::from_git_commit(&commit) {
                        Ok(commit) => {
                            if !commit.should_omit() {
                                Some(ChangelogCommit::from(commit))
                            } else {
                                None
                            }
                        }
                        Err(err) => {
                            let err = err.to_string().red();
                            warn!("{}", err);
                            None
                        }
                    })
                    .collect(),
                previous: current.map(Box::new),
            };

            current = Some(next);
        }

        current.ok_or(ChangelogError::EmptyRelease)
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
