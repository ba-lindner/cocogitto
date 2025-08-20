use crate::command::bump::{BumpData, BumpOptions};

use crate::conventional::changelog::release::Release;
use crate::conventional::changelog::template::{
    MonoRepoContext, PackageBumpContext, PackageContext,
};
use crate::conventional::changelog::ReleaseType;

use crate::conventional::version::{Increment, IncrementCommand};

use crate::git::error::TagError;
use crate::git::rev::cache::CommitInfo;
use crate::git::rev::filters::CommitRangeFilter;
use crate::git::rev::revwalk::RevwalkCache;
use crate::git::tag::Tag;
use crate::settings::{HookType, MonoRepoPackage};
use crate::{settings, CocoGitto, SETTINGS};
use anyhow::{bail, Result};

use log::{info, warn};
use tera::Tera;

#[derive(Debug)]
struct PackageBumpData<'p> {
    package_name: &'p str,
    package: &'p MonoRepoPackage,
    data: BumpData,
}

#[derive(Debug)]
pub struct PackageData<'n> {
    pub package_name: &'n str,
    pub package_path: String,
    pub version: Tag,
}

impl CocoGitto {
    pub fn create_monorepo_version(&mut self, opts: BumpOptions) -> Result<()> {
        match opts.increment {
            IncrementCommand::Auto => {
                if SETTINGS.generate_mono_repository_global_tag {
                    self.create_monorepo_version_auto(opts)
                } else {
                    if opts.annotated.is_some() {
                        warn!("--annotated flag is not supported for package bumps without a global tag");
                    }
                    self.create_all_package_version_auto(opts)
                }
            }
            _ => self.create_monorepo_version_manual(opts),
        }
    }

    pub fn create_all_package_version_auto(&mut self, opts: BumpOptions) -> Result<()> {
        self.pre_bump_checks(opts.skip_untracked)?;
        // Get package bumps
        let bumps = self.get_packages_bumps(&opts, &mut RevwalkCache::new())?;

        if bumps.is_empty() {
            print!("No conventional commits found for your packages that required a bump. Changelogs will be updated on the next bump.\nPre-Hooks and Post-Hooks have been skipped.\n");
            return Ok(());
        }

        if opts.dry_run {
            for bump in bumps {
                println!("{}", bump.data.next)
            }
            return Ok(());
        }

        let empty_bump = BumpData {
            current: Tag::default(),
            next: Tag::default(),
            release: Release::empty(),
        };
        let hook_result = self.run_hooks(HookType::PreBump, &empty_bump, opts.hooks_config, None);

        let disable_bump_commit = opts.disable_bump_commit || SETTINGS.disable_bump_commit;

        self.repository.add_all()?;
        self.unwrap_or_stash_and_exit(&Tag::default(), hook_result);
        self.bump_packages(opts.hooks_config, &bumps)?;

        if !disable_bump_commit {
            let sign = self.repository.gpg_sign();
            if opts.skip_ci || opts.skip_ci_override.is_some() {
                let skip_ci_pattern = opts.skip_ci_override.unwrap_or(SETTINGS.skip_ci.clone());
                self.repository.commit(
                    &format!("chore(version): bump packages {skip_ci_pattern}"),
                    sign,
                    true,
                )?;
            } else {
                self.repository
                    .commit("chore(version): bump packages", sign, true)?;
            }
        }

        if SETTINGS.generate_mono_repository_package_tags {
            for bump in &bumps {
                self.repository
                    .create_tag(&bump.data.next, disable_bump_commit)?;
            }
        }

        // Run per package post hooks
        for bump in bumps {
            self.run_hooks(
                HookType::PostBump,
                &bump.data,
                opts.hooks_config,
                Some((bump.package_name, bump.package)),
            )?;
        }

        // Run global post hooks
        self.run_hooks(HookType::PreBump, &empty_bump, opts.hooks_config, None)?;

        Ok(())
    }

    fn create_monorepo_version_auto(&mut self, mut opts: BumpOptions) -> Result<()> {
        self.pre_bump_checks(opts.skip_untracked)?;
        let mut cache = RevwalkCache::new();
        // Get package bumps
        let bumps = self.get_packages_bumps(&opts, &mut cache)?;
        if bumps.is_empty() {
            print!("No conventional commits found for your packages that required a bump. Changelogs will be updated on the next bump.\nPre-Hooks and Post-Hooks have been skipped.\n");
            return Ok(());
        }

        opts.increment = if SETTINGS.generate_mono_repository_package_tags {
            // Get the greatest package increment among public api packages
            IncrementCommand::AutoMonoRepoGlobal(
                bumps
                    .iter()
                    .filter(|bump| bump.package.public_api)
                    .map(|bump| bump.data.increment())
                    .max(),
            )
        } else {
            IncrementCommand::Auto
        };
        let global_bump =
            self.get_next_version(&opts, CommitRangeFilter::Global, Some(&mut cache), true)?;
        drop(cache);

        if opts.dry_run {
            for bump in bumps {
                println!("{}", bump.data.next)
            }
            print!("{}", global_bump.next);
            return Ok(());
        }

        let mut template_context = vec![];
        for bump in &bumps {
            template_context.push(PackageBumpContext {
                package_name: bump.package_name,
                package_path: bump.package.path.display().to_string(),
                version: CommitInfo::from_tag(&bump.data.next),
                from: Some(if bump.data.current.is_zero() {
                    CommitInfo::new(self.repository.get_cache().first)
                } else {
                    CommitInfo::from_tag(&bump.data.current)
                }),
            })
        }

        if !SETTINGS.disable_changelog {
            global_bump.release.pretty_print_bump_summary()?;

            let path = settings::changelog_path();
            let template = SETTINGS.get_monorepo_changelog_template()?;

            global_bump.release.write_to_file(
                path,
                template,
                ReleaseType::MonoRepo(MonoRepoContext {
                    package_lock: false,
                    packages: template_context,
                }),
            )?;
        }

        let hook_result = self.run_hooks(HookType::PreBump, &global_bump, opts.hooks_config, None);

        self.repository.add_all()?;
        self.unwrap_or_stash_and_exit(&global_bump.next, hook_result);
        self.bump_packages(opts.hooks_config, &bumps)?;

        let disable_bump_commit = opts.disable_bump_commit || SETTINGS.disable_bump_commit;

        if !disable_bump_commit {
            let sign = self.repository.gpg_sign();
            if opts.skip_ci || opts.skip_ci_override.is_some() {
                let skip_ci_pattern = opts.skip_ci_override.unwrap_or(SETTINGS.skip_ci.clone());
                self.repository.commit(
                    &format!("chore(version): {} {}", global_bump.next, skip_ci_pattern),
                    sign,
                    true,
                )?;
            } else {
                self.repository.commit(
                    &format!("chore(version): {}", global_bump.next),
                    sign,
                    true,
                )?;
            }
        }

        if SETTINGS.generate_mono_repository_package_tags {
            for bump in &bumps {
                self.repository
                    .create_tag(&bump.data.next, disable_bump_commit)?;
            }
        }

        if let Some(msg_tmpl) = opts.annotated {
            let mut context = tera::Context::new();
            context.insert("latest", &global_bump.current.version.to_string());
            context.insert("version", &global_bump.next.version.to_string());
            let msg = Tera::one_off(&msg_tmpl, &context, false)?;
            self.repository
                .create_annotated_tag(&global_bump.next, &msg, disable_bump_commit)?;
        } else {
            self.repository
                .create_tag(&global_bump.next, disable_bump_commit)?;
        }

        // Run per package post hooks
        for bump in bumps {
            self.run_hooks(
                HookType::PostBump,
                &bump.data,
                opts.hooks_config,
                Some((bump.package_name, bump.package)),
            )?;
        }

        // Run global post hooks
        self.run_hooks(HookType::PostBump, &global_bump, opts.hooks_config, None)?;

        Ok(())
    }

    fn create_monorepo_version_manual(&mut self, opts: BumpOptions) -> Result<()> {
        self.pre_bump_checks(opts.skip_untracked)?;
        // Get package bumps
        let bumps = self.get_current_packages()?;

        let bump = self.get_next_version(&opts, CommitRangeFilter::Global, None, false)?;

        if opts.dry_run {
            print!("{}", bump.next);
            return Ok(());
        }

        let mut template_context = vec![];
        for bump in bumps {
            template_context.push(PackageBumpContext {
                package_name: bump.package_name,
                package_path: bump.package_path,
                version: CommitInfo::from_tag(&bump.version),
                from: None,
            })
        }

        if !SETTINGS.disable_changelog {
            bump.release.pretty_print_bump_summary()?;

            let path = settings::changelog_path();
            let template = SETTINGS.get_monorepo_changelog_template()?;

            bump.release.write_to_file(
                path,
                template,
                ReleaseType::MonoRepo(MonoRepoContext {
                    package_lock: true,
                    packages: template_context,
                }),
            )?;
        }

        let hook_result = self.run_hooks(HookType::PreBump, &bump, opts.hooks_config, None);

        self.repository.add_all()?;
        self.unwrap_or_stash_and_exit(&Tag::default(), hook_result);

        let disable_bump_commit = opts.disable_bump_commit || SETTINGS.disable_bump_commit;

        if !disable_bump_commit {
            let sign = self.repository.gpg_sign();

            if opts.skip_ci || opts.skip_ci_override.is_some() {
                let skip_ci_pattern = opts.skip_ci_override.unwrap_or(SETTINGS.skip_ci.clone());
                self.repository.commit(
                    &format!("chore(version): {} {}", bump.next, skip_ci_pattern),
                    sign,
                    true,
                )?;
            } else {
                self.repository
                    .commit(&format!("chore(version): {}", bump.next), sign, true)?;
            }
        }

        if let Some(msg_tmpl) = opts.annotated {
            let mut context = tera::Context::new();
            context.insert("latest", &bump.current.version.to_string());
            context.insert("version", &bump.next.version.to_string());
            let msg = Tera::one_off(&msg_tmpl, &context, false)?;
            self.repository
                .create_annotated_tag(&bump.next, &msg, disable_bump_commit)?;
        } else {
            self.repository
                .create_tag(&bump.next, disable_bump_commit)?;
        }

        // Run global post hooks
        self.run_hooks(HookType::PostBump, &bump, opts.hooks_config, None)?;

        Ok(())
    }

    pub fn get_current_packages(&self) -> Result<Vec<PackageData<'static>>> {
        let mut packages = vec![];
        for (package_name, package) in SETTINGS.packages.iter() {
            let tag = match self.repository.get_latest_package_tag(package_name) {
                Ok(tag) => tag,
                Err(TagError::NoTag) => Tag::default(),
                Err(other) => bail!(other),
            };
            packages.push(PackageData {
                package_name,
                package_path: package.path.to_string_lossy().to_string(),
                version: tag,
            })
        }

        Ok(packages)
    }

    // Calculate all package bump
    fn get_packages_bumps<'c>(
        &'c self,
        opts: &BumpOptions,
        cache: &mut RevwalkCache<'c>,
    ) -> Result<Vec<PackageBumpData<'static>>> {
        let mut package_bumps = vec![];
        let mut packages: Vec<(&String, &MonoRepoPackage)> = SETTINGS.packages.iter().collect();
        packages.sort_by(|a, b| a.1.bump_order.cmp(&b.1.bump_order));

        for (package_name, package) in packages {
            let bump = self.get_next_version(
                opts,
                CommitRangeFilter::Package(package_name),
                Some(cache),
                true,
            )?;
            if bump.release.commits.is_empty() || bump.increment() == Increment::NoBump {
                continue;
            }

            package_bumps.push(PackageBumpData {
                package_name,
                package,
                data: bump,
            })
        }

        Ok(package_bumps)
    }

    // Run pre hooks and generate changelog for each package and git add the generated content
    fn bump_packages(
        &mut self,
        hooks_config: Option<&str>,
        package_bumps: &Vec<PackageBumpData>,
    ) -> Result<()> {
        for bump in package_bumps {
            bump.data.release.pretty_print_bump_summary()?;

            let path = bump.package.changelog_path();
            let template = SETTINGS.get_package_changelog_template()?;

            let additional_context = ReleaseType::Package(PackageContext {
                package_name: bump.package_name,
            });

            bump.data
                .release
                .write_to_file(&path, template, additional_context)?;
            info!("\tChangelog updated {:?}", path);

            let hook_result = self.run_hooks(
                HookType::PreBump,
                &bump.data,
                hooks_config,
                Some((bump.package_name, bump.package)),
            );

            self.repository.add_all()?;
            self.unwrap_or_stash_and_exit(&bump.data.next, hook_result);
        }

        Ok(())
    }
}
