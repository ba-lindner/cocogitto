use crate::conventional::changelog::release::Release;
use crate::conventional::commit::Commit;
use crate::git::error::TagError;
use crate::git::oid::ReleaseVersion;

use crate::conventional::error::BumpError as ConvBumpError;
use crate::conventional::version::{Increment, IncrementCommand};
use crate::git::repository::Repository;
use crate::git::rev::filters::CommitFilter;
use crate::git::rev::revspec::RevSpecPattern2;
use crate::git::tag::Tag;
use crate::hook::{Hook, Hooks};
use crate::settings::{HookType, MonoRepoPackage, Settings};
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
mod standard;

type WithPkg<'p> = (&'p str, &'p MonoRepoPackage);

trait MaybePkg: Copy {
    fn maybe_pkg(&self) -> Option<WithPkg<'_>>;
}

impl MaybePkg for () {
    fn maybe_pkg(&self) -> Option<WithPkg<'_>> {
        None
    }
}

impl MaybePkg for WithPkg<'_> {
    fn maybe_pkg(&self) -> Option<WithPkg<'_>> {
        Some(*self)
    }
}

#[derive(Default, Clone)]
pub struct BumpOptions<'a, P = ()> {
    pub increment: IncrementCommand,
    pub pre_release: Option<&'a str>,
    pub build: Option<&'a str>,
    pub hooks_config: Option<&'a str>,
    pub annotated: Option<String>,
    pub dry_run: bool,
    pub skip_ci: bool,
    pub skip_ci_override: Option<String>,
    pub skip_untracked: bool,
    pub disable_bump_commit: bool,
    pub include_packages: bool,
    pub package: P,
}

struct BumpResult<P = ()> {
    current: Tag,
    next: Tag,
    had_commits: bool,
    package: P,
}

impl<P> BumpResult<P> {
    fn no_change(&self) -> bool {
        self.current.version == self.next.version
    }

    fn increment(&self) -> Increment {
        self.next
            .get_increment_from(&self.current)
            .unwrap_or(Increment::NoBump)
    }
}

impl<'a, P> BumpOptions<'a, P> {
    fn get_new_version(
        &self,
        repository: &Repository,
        allow_empty: bool,
        increment: Option<IncrementCommand>,
    ) -> Result<BumpResult<P>>
    where
        P: MaybePkg,
    {
        let package = self.package.maybe_pkg().unzip().0;
        let current = match repository.get_latest_tag(package, false) {
            Ok(tag) => tag,
            Err(TagError::NoTag) => Tag {
                package: package.map(ToString::to_string),
                ..Default::default()
            },
            Err(other) => bail!(other),
        };
        let current_prerelease = repository
            .get_latest_tag(package, true)
            .ok()
            .filter(|tag| *tag > current);

        let increment = increment.unwrap_or_else(|| self.increment.clone());
        let (mut next, had_commits) = match current.bump(increment, repository) {
            Ok(tag) => (tag, true),
            Err(ConvBumpError::NoCommitFound) if allow_empty => {
                (current.clone().reset_metadata(), false)
            }
            Err(other) => bail!(other),
        };
        assert_eq!(next.package, current.package);

        // if prerelease exists, ensure the new tag is not smaller
        if let Some(pre_release) = &current_prerelease {
            if next < *pre_release {
                next.version.major = pre_release.version.major;
                next.version.minor = pre_release.version.minor;
                next.version.patch = pre_release.version.patch;
            }
        }

        if current.version != next.version {
            if let Some(pre_release) = self.pre_release {
                next.version.pre = Prerelease::new(pre_release)?;
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

        Ok(BumpResult {
            current,
            next,
            had_commits,
            package: self.package,
        })
    }

    pub fn set_package<P2>(self, package: P2) -> BumpOptions<'a, P2> {
        BumpOptions {
            increment: self.increment,
            pre_release: self.pre_release,
            build: self.build,
            hooks_config: self.hooks_config,
            annotated: self.annotated,
            dry_run: self.dry_run,
            skip_ci: self.skip_ci,
            skip_ci_override: self.skip_ci_override,
            skip_untracked: self.skip_untracked,
            disable_bump_commit: self.disable_bump_commit,
            include_packages: self.include_packages,
            package,
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

    pub fn unwrap_or_stash_and_exit<T>(&mut self, tag: &Tag, result: Result<T>) -> T {
        match result {
            Ok(res) => res,
            Err(err) => {
                self.repository
                    .stash_failed_version(tag.clone())
                    .expect("stash");
                error!(
                    "{}",
                    BumpError {
                        cause: err.to_string(),
                        version: tag.to_string(),
                        stash_number: 0,
                    }
                );

                exit(1);
            }
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
        tag: Tag,
    ) -> Result<Release<'_>> {
        let commit_range = self.repository.revwalk(pattern)?;
        let mut release = Release::try_from(commit_range)?;
        release.version = tag.into();
        Ok(release)
    }

    /// The target package version is not created yet when generating the changelog.
    pub fn get_package_changelog_with_target_version(
        &self,
        pattern: RevSpecPattern2,
        tag: Tag,
        package: &str,
    ) -> Result<Release<'_>> {
        let commit_range = self
            .repository
            .revwalk(pattern)?
            .apply_filter(CommitFilter::Package(package))?;

        let mut release = Release::try_from(commit_range)?;
        release.version = tag.into();
        Ok(release)
    }

    /// The target global monorepo version is not created yet when generating the changelog.
    pub fn get_monorepo_global_changelog_for_version(
        &self,
        pattern: RevSpecPattern2,
        from: ReleaseVersion,
        tag: Tag,
    ) -> Result<Release<'_>> {
        let commit_range = self
            .repository
            .revwalk(pattern)?
            .apply_filter(CommitFilter::Global)?;

        let release = match Release::try_from(commit_range) {
            Ok(mut release) => {
                release.version = tag.into();
                release
            }
            Err(_) => Release {
                version: tag.into(),
                from,
                date: Default::default(),
                commits: vec![],
                previous: None,
            },
        };

        Ok(release)
    }

    fn run_hooks<P: MaybePkg>(
        &self,
        bump: Option<&BumpResult<P>>,
        hook_type: HookType,
        hook_profile: Option<&str>,
    ) -> Result<()> {
        let settings = Settings::get(&self.repository)?;

        let package = bump.and_then(|bump| bump.package.maybe_pkg());
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

        for hook in hooks {
            let command = if let Some(bump) = bump {
                hook.insert_versions(Some(&bump.current), Some(&bump.next))?
            } else {
                hook.insert_versions(None, None)?
            };
            if command.chars().count() > 78 {
                info!("[{}...]", &command[0..78]);
            } else {
                info!("[{}]", &*command);
            };
            let package_path = package.map(|(_, p)| p.path.as_path());
            command.run(package_path).with_context(|| command.clone())?;
            println!();
        }

        Ok(())
    }
}

impl Release<'_> {
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
