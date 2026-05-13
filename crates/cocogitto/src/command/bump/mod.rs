use crate::command::bump::prerelease::increment_prerelease;
use crate::conventional::changelog::release::Release;
use crate::conventional::commit::Commit;
use crate::git::error::TagError;

use crate::conventional::error::BumpError as ConvBumpError;
use crate::conventional::version::IncrementCommand;
use crate::conventional::version::PreCommand;
use crate::git::repository::Repository;
use crate::git::rev::revspec::RevSpecPattern2;
use crate::git::tag::Tag;
use crate::hook::{Hook, HookVersion, Hooks};
use crate::settings::{HookType, MonoRepoPackage, Settings};
use crate::target::Target;
use crate::BumpError;
use crate::{CocoGitto, COMMITS_METADATA, SETTINGS};
use anyhow::Result;
use anyhow::{bail, ensure, Context};
use colored::Colorize;
use conventional_commit_parser::commit::CommitType;
use globset::Glob;
use itertools::Itertools;
use log::{error, info, warn};
use semver::{BuildMetadata, Prerelease};
use std::default::Default;
use std::fmt;
use std::fmt::Write;
use std::process::exit;

mod monorepo;
mod package;
mod prerelease;
mod standard;

#[derive(Default)]
pub struct BumpOptions<'a> {
    pub increment: IncrementCommand,
    pub pre_release: Option<PreCommand<'a>>,
    pub build: Option<&'a str>,
    pub hooks_config: Option<&'a str>,
    pub annotated: Option<String>,
    pub dry_run: bool,
    pub skip_ci: bool,
    pub skip_ci_override: Option<String>,
    pub skip_untracked: bool,
    pub disable_bump_commit: bool,
    pub include_packages: bool,
}

#[derive(Default)]
pub struct PackageBumpOptions<'a> {
    pub package_name: &'a str,
    pub package: &'a MonoRepoPackage,
    pub increment: IncrementCommand,
    pub pre_release: Option<PreCommand<'a>>,
    pub build: Option<&'a str>,
    pub hooks_config: Option<&'a str>,
    pub annotated: Option<String>,
    pub dry_run: bool,
    pub skip_ci: bool,
    pub skip_ci_override: Option<String>,
    pub skip_untracked: bool,
    pub disable_bump_commit: bool,
}

#[derive(Debug)]
struct BumpResult {
    current: Tag,
    next: Tag,
    had_commits: bool,
}

impl BumpResult {
    fn no_change(&self) -> bool {
        self.current.version == self.next.version
    }
}

impl<'a> BumpOptions<'a> {
    fn get_new_version(
        &self,
        repository: &Repository,
        package: Option<&str>,
        allow_empty: bool,
        increment: Option<IncrementCommand>,
    ) -> Result<BumpResult> {
        let current = match repository.get_latest_tag(package, false) {
            Ok(tag) => tag,
            Err(TagError::NoTag) => Tag::default(),
            Err(other) => bail!(other),
        };
        let current_prerelease = repository
            .get_latest_tag(package, true)
            .ok()
            .filter(|tag| *tag > current);

        let increment = increment.unwrap_or_else(|| self.increment.clone());
        let (mut next, had_commits) = match current.bump(increment, repository) {
            Ok(tag) => (tag, true),
            Err(ConvBumpError::NoCommitFound) if allow_empty => (current.strip_metadata(), false),
            Err(other) => bail!(other),
        };

        // if prerelease exists, ensure the new tag is not smaller
        if let Some(pre_release) = &current_prerelease {
            if next < *pre_release {
                next.version.major = pre_release.version.major;
                next.version.minor = pre_release.version.minor;
                next.version.patch = pre_release.version.patch;
            }
        }

        if current.version != next.version {
            match self.pre_release {
                Some(PreCommand::Exact(pre)) => {
                    next.version.pre = Prerelease::new(pre)?;
                }
                Some(PreCommand::Auto(pattern)) => {
                    let pre = increment_prerelease(&current_prerelease, &next, pattern)?;
                    next.version.pre = Prerelease::new(&pre)?;
                }
                None => {}
            }

            if let Some(build) = self.build {
                next.version.build = BuildMetadata::new(build)?;
            }
        }

        // ensure version doesn't decrease
        if next < current {
            bail!(
                "{}:\n\t{} version MUST be greater than current one: {}\n",
                "SemVer Error".red(),
                "cause:".red(),
                format!("{next} <= {current}").red(),
            );
        }

        next.package = package.map(ToString::to_string);

        Ok(BumpResult {
            current,
            next,
            had_commits,
        })
    }
}

impl<'a> PackageBumpOptions<'a> {
    fn common(&self) -> BumpOptions<'a> {
        BumpOptions {
            increment: self.increment.clone(),
            pre_release: self.pre_release.clone(),
            build: self.build,
            hooks_config: self.hooks_config,
            annotated: self.annotated.clone(),
            dry_run: self.dry_run,
            skip_ci: self.skip_ci,
            skip_ci_override: self.skip_ci_override.clone(),
            skip_untracked: self.skip_untracked,
            disable_bump_commit: self.disable_bump_commit,
            include_packages: false,
        }
    }
}

impl CocoGitto {
    fn get_bump_revspec(&mut self, current_tag: &Tag) -> RevSpecPattern2 {
        if current_tag.is_zero() {
            RevSpecPattern2::full()
        } else {
            // this function is always called with the latest tag, so it should always have a oid
            RevSpecPattern2::from(*current_tag.oid_unchecked())
        }
    }

    fn pre_bump_checks(&mut self, skip_untracked: bool) -> Result<()> {
        if *SETTINGS == Settings::default() {
            let part1 = "Warning: using".yellow();
            let part2 = "with the default configuration. \n".yellow();
            let part3 = "You may want to create a".yellow();
            let part4 = "file in your project root to configure bumps.\n".yellow();
            warn!(
                "{} 'cog bump' {}{} 'cog.toml' {}",
                part1, part2, part3, part4
            );
        }
        let statuses = self.repository.get_statuses()?;

        if skip_untracked || SETTINGS.skip_untracked {
            eprintln!("{}", self.repository.get_statuses()?);
        } else {
            ensure!(statuses.0.is_empty(), "{}", self.repository.get_statuses()?);
        }

        if !SETTINGS.branch_whitelist.is_empty() {
            if let Some(branch) = self.repository.get_branch_shorthand() {
                let whitelist = &SETTINGS.branch_whitelist;
                let is_match = whitelist.iter().any(|pattern| {
                    let glob = Glob::new(pattern)
                        .expect("invalid glob pattern")
                        .compile_matcher();
                    glob.is_match(&branch)
                });

                ensure!(
                    is_match,
                    "No patterns matched in {:?} for branch '{}', bump is not allowed",
                    whitelist,
                    branch
                )
            }
        };

        Ok(())
    }

    /// The target version is not created yet when generating the changelog.
    pub fn get_changelog_with_target_version(
        &self,
        pattern: RevSpecPattern2,
        target: Target,
        tag: Tag,
    ) -> Result<Release> {
        let allow_empty = matches!(target, Target::Monorepo { .. });
        let commit_range = self.repository.revwalk(pattern)?.for_target(target)?;
        let fallback_from = commit_range.version_range().0;
        let release = match Release::try_from(commit_range) {
            Ok(mut release) => {
                release.version = tag.into();
                release
            }
            Err(_) if allow_empty => Release {
                version: tag.into(),
                from: fallback_from,
                date: Default::default(),
                commits: vec![],
                previous: None,
            },
            Err(why) => bail!(why),
        };
        Ok(release)
    }

    fn run_hooks(
        &mut self,
        bump: Option<&BumpResult>,
        target: Target,
        hook_type: HookType,
        hook_profile: Option<&str>,
    ) -> Result<()> {
        let hook_result = self.run_hooks_impl(bump, target, hook_type, hook_profile);

        if let HookType::PostBump = hook_type {
            return hook_result;
        }
        self.repository.add_all()?;
        if let Err(err) = hook_result {
            let tag = bump.map(|bump| bump.next.clone()).unwrap_or_default();
            error!(
                "{}",
                BumpError {
                    cause: err.to_string(),
                    version: tag.to_string(),
                    stash_number: 0,
                }
            );
            self.repository
                .stash_failed_version(tag)
                .expect("failed to stash bump hook changes");

            exit(1);
        };

        Ok(())
    }

    fn run_hooks_impl(
        &self,
        bump: Option<&BumpResult>,
        target: Target,
        hook_type: HookType,
        hook_profile: Option<&str>,
    ) -> Result<()> {
        let settings = Settings::get(&self.repository)?;

        let package = if let Target::Package { name, package } = target {
            Some((name, package))
        } else {
            None
        };
        let (hook_src, package_hint) = package
            .map(|(name, package)| (package as &dyn Hooks, format!(" for package {name}")))
            .unwrap_or((&settings, String::new()));

        let (raw_hooks, profile_hint) = if let Some(profile) = hook_profile {
            (
                hook_src.get_profile_hooks(profile, hook_type),
                format!(" bump profile {profile}"),
            )
        } else {
            (hook_src.get_hooks(hook_type), String::new())
        };

        let hooks: Vec<Hook> = raw_hooks
            .iter()
            .enumerate()
            .map(|(idx, s)| {
                s.parse().with_context(|| {
                    format!("Cannot parse{profile_hint} hook{package_hint} at index {idx}")
                })
            })
            .try_collect()?;

        if !hooks.is_empty() {
            let hook_type = match hook_type {
                HookType::PreBump => "pre-bump",
                HookType::PostBump => "post-bump",
            };

            let msg = if let Some((name, _)) = package {
                format!("[{hook_type}-{name}]")
            } else {
                format!("[{hook_type}]")
            };
            info!("{}", msg.underline().white().bold());
        }

        let current_version = bump
            .map(|bump| HookVersion::new(bump.current.clone()))
            .filter(|vers| !vers.prefixed_tag.is_zero());
        let next_version = bump.map(|bump| HookVersion::new(bump.next.clone()));
        for mut hook in hooks {
            hook.insert_versions(current_version.as_ref(), next_version.as_ref())?;
            let command = hook.to_string();
            let command = if command.chars().count() > 78 {
                &command[0..command.len()]
            } else {
                &command
            };
            info!("[{command}]");
            let package_path = package.map(|p| p.1.path.as_path());
            hook.run(package_path).context(hook.to_string())?;
            println!();
        }

        Ok(())
    }
}

impl Release {
    fn pretty_print_bump_summary(&self) -> Result<(), fmt::Error> {
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
