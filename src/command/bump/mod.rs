use crate::conventional::changelog::release::Release;
use crate::conventional::changelog::renderer::Renderer;
use crate::conventional::changelog::template::TplTarget;
use crate::conventional::changelog::ReleaseType;
use crate::conventional::commit::Commit;
use crate::git::error::TagError;

use crate::conventional::error::BumpError as ConvBumpError;
use crate::conventional::version::{Increment, IncrementCommand};
use crate::git::repository::Repository;
use crate::git::rev::filters::CommitFilter;
use crate::git::tag::Tag;
use crate::hook::{Hook, Hooks};
use crate::settings::{self, HookType, MonoRepoPackage, Settings};
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
    release: Release<'static>,
    package: P,
}

impl<P> BumpResult<P> {
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
        let bump_filter = match &increment {
            IncrementCommand::AutoMonoRepoGlobal(_) => CommitFilter::Global,
            IncrementCommand::AutoPackage(package) => CommitFilter::Package(package),
            _ => CommitFilter::Standard,
        };
        let mut commits = repository
            .commits_since_tag(&current)?
            .apply_filter(bump_filter)?;
        let mut next = match current.bump(&increment, &commits) {
            Ok(tag) => tag,
            Err(ConvBumpError::NoCommitFound) if allow_empty => current.clone().reset_metadata(),
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

        if let Some(prerelease) = &current_prerelease {
            if !next.version.pre.is_empty() {
                commits = repository.commits_since_tag(prerelease)?;
            }
        }
        commits = commits.apply_filter(if let Some(package) = package {
            CommitFilter::Package(package)
        } else if SETTINGS.packages.is_empty() {
            CommitFilter::Standard
        } else {
            CommitFilter::Global
        })?;
        let mut release = Release::build(&commits).unwrap_or(Release {
            version: next.clone().into(),
            from: current.clone().into(),
            date: Default::default(),
            commits: Vec::new(),
            packages: Vec::new(),
        });
        release.version = next.clone().into();

        Ok(BumpResult {
            current,
            next,
            release,
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

    fn render_changelog<P: MaybePkg>(&self, bump: &BumpResult<P>, kind: ReleaseType) -> Result<()> {
        if SETTINGS.disable_changelog {
            return Ok(());
        }

        bump.release.pretty_print_bump_summary()?;

        let template = SETTINGS.get_changelog_template(match kind {
            ReleaseType::Standard => TplTarget::Standard,
            ReleaseType::MonoRepo(_) => TplTarget::Monorepo,
            ReleaseType::Package(_) => TplTarget::Package,
        })?;
        let mut renderer = Renderer::try_new(template)?;
        if let Some(context) = SETTINGS.get_template_context() {
            renderer.add_context(context);
        }
        let path = bump
            .package
            .maybe_pkg()
            .map(|(_, package)| package.changelog_path())
            .unwrap_or(settings::changelog_path().clone());
        bump.release.write_to_file(path, kind, renderer)?;

        Ok(())
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

    fn create_bump_commit<P>(&self, opts: &BumpOptions<P>, msg: impl ToString) -> Result<()> {
        if opts.disable_bump_commit || SETTINGS.disable_bump_commit {
            return Ok(());
        }

        let skip_ci = opts
            .skip_ci_override
            .as_deref()
            .or(opts.skip_ci.then_some(&SETTINGS.skip_ci));
        let message = if let Some(skip_ci) = skip_ci {
            format!("chore(version): {} {skip_ci}", msg.to_string())
        } else {
            format!("chore(version): {}", msg.to_string())
        };

        let sign = self.repository.gpg_sign();
        self.repository.commit(&message, sign, true)?;

        Ok(())
    }

    fn create_bump_tag<P>(&self, bump: &BumpResult<P>, annotated: Option<&str>) -> Result<()> {
        let head = self.repository.get_head_commit()?.into_object();
        let tag = bump.next.to_string();

        if let Some(msg_tmpl) = annotated {
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
