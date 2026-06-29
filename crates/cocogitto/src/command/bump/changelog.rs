use anyhow::{bail, Result};
use colored::Colorize as _;
use conventional_commit_parser::commit::CommitType;
use itertools::Itertools as _;
use log::info;

use crate::command::bump::BumpResult;
use crate::conventional::changelog::context::{
    MonoRepoContext, PackageBumpContext, PackageContext,
};
use crate::conventional::changelog::release::Release;
use crate::conventional::changelog::ReleaseType;
use crate::conventional::commit::Commit;
use crate::git::oid::ReleaseVersion;
use crate::git::rev::revspec::RevSpecPattern2;
use crate::git::tag::Tag;
use crate::target::Target;
use crate::{settings, CocoGitto, COMMITS_METADATA, SETTINGS};

use std::fmt::Write as _;

impl CocoGitto {
    /// The target version is not created yet when generating the changelog.
    pub fn get_changelog_with_target_version(
        &self,
        target: Target,
        from: &Tag,
        version: &Tag,
    ) -> Result<Release> {
        let allow_empty = matches!(target, Target::Monorepo { .. });
        let pattern = if from.is_zero() {
            RevSpecPattern2::full()
        } else {
            RevSpecPattern2 {
                from: from.oid,
                to: None,
            }
        };
        let version: ReleaseVersion = version.clone().into();
        let commit_range = self.repository.revwalk(pattern)?.for_target(target)?;
        let fallback_from = commit_range.version_range().0;
        let release = match Release::try_from(commit_range) {
            Ok(mut release) => {
                release.version = version;
                release
            }
            Err(_) if allow_empty => Release {
                version,
                from: fallback_from,
                date: Default::default(),
                commits: vec![],
                previous: None,
                packages: Vec::new(),
            },
            Err(why) => bail!(why),
        };
        Ok(release)
    }

    pub(super) fn render_changelog(
        &self,
        bump: &BumpResult,
        packages: Option<Vec<PackageBumpContext<'_>>>,
    ) -> Result<()> {
        if SETTINGS.disable_changelog {
            return Ok(());
        }

        let (template, kind, path) = match bump.target {
            Target::Standard => (
                SETTINGS.get_changelog_template()?,
                ReleaseType::Standard,
                settings::changelog_path().to_owned(),
            ),
            Target::Package { name, package } => (
                SETTINGS.get_package_changelog_template()?,
                ReleaseType::Package(PackageContext { package_name: name }),
                package.changelog_path(),
            ),
            Target::Monorepo { manual } | Target::Unified { manual } => (
                SETTINGS.get_monorepo_changelog_template()?,
                ReleaseType::MonoRepo(MonoRepoContext {
                    package_lock: manual,
                    packages: packages.unwrap_or_default(),
                }),
                settings::changelog_path().to_owned(),
            ),
        };

        let changelog =
            self.get_changelog_with_target_version(bump.target, &bump.current, &bump.next)?;

        changelog.pretty_print_bump_summary()?;

        changelog.write_to_file(&path, template, kind)?;

        info!("\tChangelog updated {:?}", path);

        Ok(())
    }
}

impl Release {
    pub(super) fn pretty_print_bump_summary(&self) -> Result<(), std::fmt::Error> {
        let conventional_commits: Vec<&Commit> = self
            .commits
            .iter()
            .map(|ch_commit| &ch_commit.commit)
            .collect();

        // Commits which type are neither feat, fix nor breaking changes
        // won't affect the version number.
        let mut non_bump_commits: Vec<&CommitType> = conventional_commits
            .iter()
            .filter_map(|commit: &&Commit| {
                let commit_config = COMMITS_METADATA.get(&commit.conventional.commit_type);
                match commit_config {
                    Some(commit_config)
                        if commit_config.bump_minor() || commit_config.bump_patch() =>
                    {
                        None
                    }
                    _ if commit.conventional.is_breaking_change => None,
                    _ => Some(&commit.conventional.commit_type),
                }
            })
            .collect();

        non_bump_commits.sort();

        let non_bump_commits: Vec<(usize, &CommitType)> = non_bump_commits
            .into_iter()
            .dedup_by_with_count(|c1, c2| c1 == c2)
            .collect();

        if !non_bump_commits.is_empty() {
            let mut skip_message = "  Skipping irrelevant commits:\n".to_string();
            for (count, commit_type) in non_bump_commits {
                writeln!(skip_message, "    - {}: {}", commit_type.as_ref(), count)?;
            }

            info!("{}", skip_message);
        }

        let bump_commits =
            conventional_commits
                .iter()
                .filter(|commit| match &commit.conventional.commit_type {
                    CommitType::Feature | CommitType::BugFix => true,
                    _commit_type if commit.conventional.is_breaking_change => true,
                    _ => false,
                });

        for commit in bump_commits {
            match &commit.conventional.commit_type {
                _commit_type if commit.conventional.is_breaking_change => {
                    info!(
                        "\t Found {} commit {} with type: {}",
                        "BREAKING CHANGE".red(),
                        commit.shorthand().blue(),
                        commit.conventional.commit_type.as_ref().yellow()
                    )
                }
                CommitType::Feature => {
                    info!("\tFound feature commit {}", commit.shorthand().blue())
                }
                CommitType::BugFix => info!("\tFound bug fix commit {}", commit.shorthand().blue()),
                _ => (),
            }
        }

        Ok(())
    }
}
