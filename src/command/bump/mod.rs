use crate::conventional::changelog::release::Release;
use crate::conventional::commit::Commit;
use crate::git::error::TagError;

use crate::conventional::version::{Increment, IncrementCommand};
use crate::git::rev::cache::CommitInfo;
use crate::git::rev::filters::CommitRangeFilter;
use crate::git::rev::revspec::RevSpec;
use crate::git::rev::revwalk::RevwalkCache;
use crate::git::tag::{Tag, TagLookUpOptions};
use crate::hook::{Hook, HookVersion, Hooks};
use crate::settings::{HookType, MonoRepoPackage, Settings};
use crate::BumpError;
use crate::{CocoGitto, COMMITS_METADATA, SETTINGS};
use anyhow::Result;
use anyhow::{bail, ensure, Context};
use chrono::Utc;
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

#[derive(Default)]
pub struct BumpOptions<'a> {
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
}

#[derive(Default)]
pub struct PackageBumpOptions<'a> {
    pub package_name: &'a str,
    pub package: &'a MonoRepoPackage,
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
}

struct BumpResult {
    current: Tag,
    next: Tag,
    release: Release,
}

impl BumpResult {
    fn no_change(&self) -> bool {
        self.current.version == self.next.version
    }
}

impl<'a> PackageBumpOptions<'a> {
    fn common(&self) -> BumpOptions<'a> {
        BumpOptions {
            increment: self.increment.clone(),
            pre_release: self.pre_release,
            build: self.build,
            hooks_config: self.hooks_config,
            annotated: self.annotated.clone(),
            dry_run: self.dry_run,
            skip_ci: self.skip_ci,
            skip_ci_override: self.skip_ci_override.clone(),
            skip_untracked: self.skip_untracked,
            disable_bump_commit: self.disable_bump_commit,
        }
    }
}

struct HookRunOptions<'a> {
    hook_type: HookType,
    current_tag: Option<&'a HookVersion>,
    next_version: Option<&'a HookVersion>,
    hook_profile: Option<&'a str>,
    package_name: Option<&'a str>,
    package: Option<&'a MonoRepoPackage>,
}

impl<'a> HookRunOptions<'a> {
    pub fn post_bump() -> Self {
        Self {
            hook_type: HookType::PostBump,
            current_tag: None,
            next_version: None,
            hook_profile: None,
            package_name: None,
            package: None,
        }
    }

    pub fn pre_bump() -> Self {
        Self {
            hook_type: HookType::PreBump,
            current_tag: None,
            next_version: None,
            hook_profile: None,
            package_name: None,
            package: None,
        }
    }

    pub fn current_tag<'b>(mut self, version: Option<&'b HookVersion>) -> Self
    where
        'b: 'a,
    {
        self.current_tag = version;
        self
    }

    pub fn next_version<'b>(mut self, version: &'b HookVersion) -> Self
    where
        'b: 'a,
    {
        self.next_version = Some(version);
        self
    }

    pub fn hook_profile<'b>(mut self, profile: Option<&'b str>) -> Self
    where
        'b: 'a,
    {
        self.hook_profile = profile;
        self
    }

    pub fn package<'b>(mut self, name: &'b str, package: &'b MonoRepoPackage) -> Self
    where
        'b: 'a,
    {
        self.package_name = Some(name);
        self.package = Some(package);
        self
    }
}

impl CocoGitto {
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

    fn pre_bump_checks(&self, skip_untracked: bool) -> Result<()> {
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

    fn get_next_version<'c>(
        &'c self,
        opts: &'_ BumpOptions,
        filter: CommitRangeFilter<'_>,
        cache: Option<&'_ mut RevwalkCache<'c>>,
        allow_empty: bool,
    ) -> Result<BumpResult> {
        let tag_opts = match filter {
            CommitRangeFilter::Package(pkg) => TagLookUpOptions::package(pkg),
            _ => TagLookUpOptions::default(),
        };
        let current = match self.repository.get_latest_tag(tag_opts) {
            Ok(tag) => tag,
            Err(TagError::NoTag) => Tag::default(),
            Err(other) => bail!(other),
        };
        let current_prerelease = self
            .repository
            .get_latest_tag(tag_opts.include_pre_release())
            .ok()
            .filter(|tag| *tag > current);

        let spec = RevSpec {
            from: current.oid,
            to: None,
        };
        let mut iter = if let Some(cache) = cache {
            self.repository.revwalk_cached(&spec, cache)?
        } else {
            self.repository.revwalk(&spec)?
        };
        iter.apply_ignore_settings();
        let filter_after_increment =
            opts.increment == IncrementCommand::Auto && filter == CommitRangeFilter::Global;
        if !filter_after_increment {
            iter.filter(&self.repository, filter)?;
        }
        let hist_increment = iter.get_increment(current.version.major == 0);
        let hist_increment = if allow_empty {
            hist_increment.unwrap_or(Increment::NoBump)
        } else {
            hist_increment?
        };
        if filter_after_increment {
            iter.filter(&self.repository, filter)?;
        }
        let mut next = match &opts.increment {
            IncrementCommand::Auto | IncrementCommand::AutoPackage(_) => {
                current.bump_fixed(hist_increment)
            }
            IncrementCommand::Fixed(increment) => current.bump_fixed(*increment),
            IncrementCommand::AutoMonoRepoGlobal(pkg_increment) => {
                current.bump_fixed(hist_increment.max(pkg_increment.unwrap_or(Increment::NoBump)))
            }
            IncrementCommand::Manual(version) => current.manual_bump(version)?,
        };

        // if prerelease exists, ensure the new tag is not smaller
        if let Some(pre_release) = &current_prerelease {
            if next < *pre_release {
                next.version.major = pre_release.version.major;
                next.version.minor = pre_release.version.minor;
                next.version.patch = pre_release.version.patch;
            }
        }

        if let Some(pre_release) = opts.pre_release {
            next.version.pre = Prerelease::new(pre_release)?;
        }

        if let Some(build) = opts.build {
            next.version.build = BuildMetadata::new(build)?;
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

        let release = Release::try_from(iter);
        let mut release = if allow_empty {
            release.unwrap_or_else(|_| Release {
                version: CommitInfo::from_tag(&next),
                from: CommitInfo::from_tag(&current),
                date: Utc::now().naive_local(),
                commits: Vec::new(),
                previous: None,
            })
        } else {
            release?
        };
        release.version = CommitInfo::from_tag(&next);

        Ok(BumpResult {
            current,
            next,
            release,
        })
    }

    fn run_hooks(&self, options: HookRunOptions) -> Result<()> {
        let settings = Settings::get(&self.repository)?;

        let hooks: Vec<Hook> = match (options.package, options.hook_profile) {
            (None, Some(profile)) => settings
                .get_profile_hooks(profile, options.hook_type)
                .iter()
                .map(|s| s.parse())
                .enumerate()
                .map(|(idx, result)| {
                    result.context(format!(
                        "Cannot parse bump profile {profile} hook at index {idx}"
                    ))
                })
                .try_collect()?,

            (Some(package), Some(profile)) => {
                let hooks = package.get_profile_hooks(profile, options.hook_type);

                hooks
                    .iter()
                    .map(|s| s.parse())
                    .enumerate()
                    .map(|(idx, result)| {
                        result.context(format!(
                            "Cannot parse bump profile {profile} hook at index {idx}"
                        ))
                    })
                    .try_collect()?
            }
            (Some(package), None) => package
                .get_hooks(options.hook_type)
                .iter()
                .map(|s| s.parse())
                .enumerate()
                .map(|(idx, result)| result.context(format!("Cannot parse hook at index {idx}")))
                .try_collect()?,
            (None, None) => settings
                .get_hooks(options.hook_type)
                .iter()
                .map(|s| s.parse())
                .enumerate()
                .map(|(idx, result)| result.context(format!("Cannot parse hook at index {idx}")))
                .try_collect()?,
        };

        if !hooks.is_empty() {
            let hook_type = match options.hook_type {
                HookType::PreBump => "pre-bump",
                HookType::PostBump => "post-bump",
            };

            match options.package_name {
                None => {
                    let msg = format!("[{hook_type}]").underline().white().bold();
                    info!("{msg}")
                }
                Some(package_name) => {
                    let msg = format!("[{hook_type}-{package_name}]")
                        .underline()
                        .white()
                        .bold();
                    info!("{msg}")
                }
            }
        }

        for mut hook in hooks {
            hook.insert_versions(options.current_tag, options.next_version)?;
            let command = hook.to_string();
            let command = if command.chars().count() > 78 {
                &command[0..command.len()]
            } else {
                &command
            };
            info!("[{command}]");
            let package_path = options.package.map(|p| p.path.as_path());
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
