use crate::conventional::changelog::release::Release;
use crate::conventional::changelog::template::{
    Empty, MonoRepoContext, PackageBumpContext, PackageContext, ToContext,
};
use crate::conventional::changelog::Renderer;
use crate::conventional::commit::Commit;

use crate::conventional::version::{Increment, IncrementCommand};
use crate::git::rev::filters::CommitRangeFilter;
use crate::git::rev::revspec::RevSpec;
use crate::git::rev::revwalk::RevwalkCache;
use crate::git::tag::Tag;
use crate::hook::{Hook, Hooks};
use crate::settings::{HookType, MonoRepoPackage, Settings};
use crate::BumpHookError;
use crate::{CocoGitto, SETTINGS};
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
use std::process::{exit, Command};

pub type WithPkg<'p> = (&'p str, &'p MonoRepoPackage);

pub trait MaybePkg {
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

#[derive(Debug)]
struct BumpData<P = ()> {
    current: Tag,
    next: Tag,
    release: Release,
    package: P,
}

impl<P> BumpData<P> {
    fn set_package<P2>(self, package: P2) -> BumpData<P2> {
        BumpData {
            current: self.current,
            next: self.next,
            release: self.release,
            package,
        }
    }

    fn increment(&self) -> Increment {
        self.next
            .get_increment_from(&self.current)
            .unwrap_or(Increment::NoBump)
    }
}

impl CocoGitto {
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
        cache: &'_ mut RevwalkCache<'c>,
    ) -> Result<BumpData> {
        let current = self
            .repository
            .get_latest_tag(filter.as_package(), false)
            .unwrap_or_else(|| Tag {
                package: filter.as_package().map(ToString::to_string),
                ..Default::default()
            });
        let current_prerelease = self
            .repository
            .get_latest_tag(filter.as_package(), true)
            .filter(|tag| *tag > current);

        let spec = RevSpec::from_tag(&current);
        let mut range = self
            .repository
            .revwalk_cached(&spec, cache)?
            .apply_ignore_settings()
            .filter(filter)?;
        let mut next = match current.bump(&opts.increment, &range) {
            Ok(tag) => tag,
            Err(crate::conventional::error::BumpError::NoCommitFound) => {
                current.clone().reset_metadata()
            }
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

        if let (Some(_), Some(pre)) = (opts.pre_release, current_prerelease) {
            range = self
                .repository
                .revwalk_cached(&RevSpec::from_tag(&pre), cache)?
                .apply_ignore_settings()
                .filter(filter)?;
        }
        let mut release = Release::build_single(range, filter);
        if release.commits.is_empty() {
            release.from = current.clone().into();
        }
        release.version = next.clone().into();

        Ok(BumpData {
            current,
            next,
            release,
            package: (),
        })
    }

    fn render_changelog<P: MaybePkg>(
        &self,
        bump: &BumpData<P>,
        context: impl ToContext,
    ) -> Result<()> {
        if SETTINGS.disable_changelog {
            return Ok(());
        }

        bump.release.pretty_print_bump_summary()?;

        let template = if SETTINGS.packages.is_empty() {
            SETTINGS.get_changelog_template()?
        } else if bump.package.maybe_pkg().is_some() {
            SETTINGS.get_package_changelog_template()?
        } else {
            SETTINGS.get_monorepo_changelog_template()?
        };

        let mut renderer = Renderer::new(template)?;

        if let Some(remote) = SETTINGS.get_remote_context() {
            renderer = renderer.with_context(remote);
        }

        let path = match bump.package.maybe_pkg() {
            Some((package_name, package)) => {
                renderer = renderer.with_context(PackageContext { package_name });
                &package.changelog_path()
            }
            None => crate::settings::changelog_path(),
        };

        renderer
            .with_context(&*SETTINGS)
            .with_context(context)
            .render_single(&bump.release)?
            .write_to_file(path)?;

        self.repository.add_all()?;

        Ok(())
    }

    fn create_bump_commit(
        &self,
        opts: &BumpOptions,
        message: impl std::fmt::Display,
    ) -> Result<()> {
        if opts.disable_bump_commit || SETTINGS.disable_bump_commit {
            return Ok(());
        }

        let mut message = format!("chore(version): {message}");

        let skip_ci = opts
            .skip_ci_override
            .as_deref()
            .or(opts.skip_ci.then_some(&SETTINGS.skip_ci));
        if let Some(skip_ci) = skip_ci {
            message += " ";
            message += skip_ci;
        }

        let sign = self.repository.gpg_sign();
        self.repository.commit(&message, sign, true)?;

        Ok(())
    }

    fn create_bump_tag<P>(&self, bump: &BumpData<P>, opts: &BumpOptions) -> Result<()> {
        let head = self.repository.get_head_commit()?.into_object();
        let tag = bump.next.to_string();

        if let Some(msg_tmpl) = &opts.annotated {
            let mut context = tera::Context::new();
            context.insert("latest", &bump.current.version.to_string());
            context.insert("version", &bump.next.version.to_string());
            let message = tera::Tera::one_off(msg_tmpl, &context, false)?;
            let sig = self.repository.0.signature()?;
            self.repository.0.tag(&tag, &head, &sig, &message, false)?;
        } else {
            self.repository.0.tag_lightweight(&tag, &head, false)?;
        }

        Ok(())
    }

    fn run_hooks<P: MaybePkg>(
        &mut self,
        hook_type: HookType,
        bump: &BumpData<P>,
        hook_profile: Option<&str>,
    ) -> Result<()> {
        let (package_name, package) = bump.package.maybe_pkg().unzip();

        let settings = Settings::get(&self.repository)?;

        // Acquire hooks to be executed
        let hook_src = package
            .map(|package| package as &dyn Hooks)
            .unwrap_or(&settings);
        let hooks = if let Some(profile) = hook_profile {
            hook_src.get_profile_hooks(profile, hook_type)
        } else {
            hook_src.get_hooks(hook_type)
        };
        if hooks.is_empty() {
            return Ok(());
        }

        // print some info for user
        let hook_type_str = match hook_type {
            HookType::PreBump => "pre-bump",
            HookType::PostBump => "post-bump",
        };
        let msg = if let Some(name) = package_name {
            format!("[{hook_type_str}-{name}]")
        } else {
            format!("[{hook_type_str}]")
        };
        info!("{}", msg.underline().white().bold());

        // execute hooks
        let profile_hint = hook_profile
            .map(|profile| format!(" bump profile {profile}"))
            .unwrap_or_default();
        let exec_result = (|| -> Result<()> {
            for (idx, hook) in hooks.iter().enumerate() {
                let hook: Hook = hook
                    .parse()
                    .with_context(|| format!("Cannot parse{profile_hint} hook at index {idx}"))?;

                let command = hook.insert_versions(&bump.current, &bump.next)?;
                if command.len() > 80 {
                    info!("[{}...]", command.chars().take(77).collect::<String>())
                } else {
                    info!("[{command}]");
                };
                let mut cmd = Command::new("sh");
                cmd.args(["-c", &command]);
                if let Some(package) = package {
                    cmd.current_dir(&package.path);
                }
                let status = cmd.status().context(command)?;
                ensure!(status.success(), "hook failed with status {}", status);
                println!();
            }
            Ok(())
        })();

        if hook_type == HookType::PostBump {
            return exec_result;
        }

        // for pre-bump hooks, stash & exit on error
        self.repository.add_all()?;
        if let Err(err) = exec_result {
            self.repository
                .stash_failed_version(&bump.next)
                .expect("failed to stash bump hook changes");
            error!(
                "{}",
                BumpHookError {
                    cause: err.to_string(),
                    version: bump.next.to_string(),
                    stash_number: 0,
                }
            );

            exit(1);
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
        let non_bump_commits: Vec<_> = conventional_commits
            .iter()
            .filter(|commit| commit.increment(false) == Increment::NoBump)
            .map(|commit| &commit.conventional.commit_type)
            .sorted()
            .dedup_with_count()
            .collect();

        if !non_bump_commits.is_empty() {
            let mut skip_message = "  Skipping irrelevant commits:\n".to_string();
            for (count, commit_type) in non_bump_commits {
                writeln!(skip_message, "    - {}: {}", commit_type.as_ref(), count)?;
            }

            info!("{}", skip_message);
        }

        let bump_commits = conventional_commits
            .iter()
            .filter(|commit| commit.increment(false) != Increment::NoBump);

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

#[derive(Debug, Clone)]
pub struct BumpTargets {
    pub standard: bool,
    pub global: bool,
    pub pkg: Vec<String>,
}

impl BumpTargets {
    pub fn standard() -> Self {
        Self {
            standard: true,
            global: false,
            pkg: Vec::new(),
        }
    }

    pub fn monorepo() -> Self {
        Self {
            standard: false,
            global: true,
            pkg: SETTINGS.packages.keys().cloned().collect(),
        }
    }

    pub fn monorepo_global() -> Self {
        Self {
            standard: false,
            global: true,
            pkg: Vec::new(),
        }
    }

    pub fn all_packages() -> Self {
        Self {
            standard: false,
            global: false,
            pkg: SETTINGS.packages.keys().cloned().collect(),
        }
    }

    pub fn package(pkg: impl Into<String>) -> Self {
        Self {
            standard: false,
            global: false,
            pkg: vec![pkg.into()],
        }
    }
}

impl CocoGitto {
    pub fn create_version(&mut self, mut opts: BumpOptions, targets: &BumpTargets) -> Result<()> {
        self.pre_bump_checks(opts.skip_untracked)?;
        let mut cache = RevwalkCache::new();

        let mut pkg_bumps = targets
            .pkg
            .iter()
            .map(|package_name: &String| {
                let package = &SETTINGS.packages[package_name];
                self.get_next_version(&opts, CommitRangeFilter::Package(package_name), &mut cache)
                    .map(|bump| bump.set_package((package_name.as_str(), package)))
            })
            .filter_ok(|bump| bump.increment() > Increment::NoBump)
            .collect::<Result<Vec<_>, _>>()?;
        pkg_bumps.sort_by_key(|bump| bump.package.1.bump_order);
        let pkg_bumps = pkg_bumps;

        if opts.increment == IncrementCommand::Auto {
            opts.increment = IncrementCommand::AutoMonoRepoGlobal(
                pkg_bumps
                    .iter()
                    .filter(|bump| bump.package.1.public_api)
                    .map(|bump| bump.increment())
                    .max(),
            )
        }

        let global_filter = if targets.standard {
            Some(CommitRangeFilter::None)
        } else if targets.global {
            Some(CommitRangeFilter::Global)
        } else {
            None
        };
        let global_bump = global_filter
            .map(|filter| self.get_next_version(&opts, filter, &mut cache))
            .transpose()?
            .filter(|bump| bump.increment() > Increment::NoBump);
        drop(cache);

        if pkg_bumps.is_empty() && global_bump.is_none() {
            print!(
                "No conventional commits found that required a bump. Changelogs will be updated on the next bump.\nPre-Hooks and Post-Hooks have been skipped.\n"
            );
            return Ok(());
        }

        if opts.dry_run {
            for bump in pkg_bumps {
                println!("{}", bump.next)
            }
            if let Some(bump) = global_bump {
                println!("{}", bump.next);
            }
            return Ok(());
        }

        if let Some(bump) = &global_bump {
            self.render_changelog(
                bump,
                MonoRepoContext {
                    package_lock: matches!(opts.increment, IncrementCommand::AutoMonoRepoGlobal(_)),
                    packages: pkg_bumps
                        .iter()
                        .map(|bump| PackageBumpContext {
                            package_name: bump.package.0,
                            package_path: bump.package.1.path.display().to_string(),
                            version: bump.next.clone().into(),
                            from: Some(if bump.current.is_zero() {
                                self.repository.get_cache().first.into()
                            } else {
                                bump.current.clone().into()
                            }),
                        })
                        .collect(),
                },
            )?;
            self.run_hooks(HookType::PreBump, &bump, opts.hooks_config)?;
        }

        for bump in &pkg_bumps {
            self.render_changelog(bump, Empty)?;
            self.run_hooks(HookType::PreBump, bump, opts.hooks_config)?;
        }

        if let Some(bump) = &global_bump {
            self.create_bump_commit(&opts, &bump.next)?;
            self.create_bump_tag(bump, &opts)?;
        } else {
            self.create_bump_commit(&opts, "bump packages")?;
        }

        if SETTINGS.generate_mono_repository_package_tags {
            for bump in &pkg_bumps {
                self.create_bump_tag(bump, &opts)?;
            }
        }

        for bump in &pkg_bumps {
            self.run_hooks(HookType::PostBump, bump, opts.hooks_config)?;
        }

        if let Some(bump) = &global_bump {
            self.run_hooks(HookType::PostBump, bump, opts.hooks_config)?;
        }

        Ok(())
    }
}
