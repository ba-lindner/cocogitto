use std::path::Path;

use crate::command::bump::{BumpOptions, BumpResult, WithPkg};

use crate::conventional::changelog::template::{
    MonoRepoContext, PackageBumpContext, PackageContext,
};
use crate::conventional::changelog::ReleaseType;

use crate::conventional::version::IncrementCommand;

use crate::git::error::TagError;
use crate::git::tag::Tag;
use crate::settings::{HookType, MonoRepoPackage};
use crate::{settings, CocoGitto, SETTINGS};
use anyhow::{bail, Result};

use log::{info, warn};

use crate::git::oid::ReleaseVersion;

#[derive(Debug)]
pub struct PackageData<'p> {
    pub package_name: &'p str,
    pub package_path: &'p Path,
    pub version: Tag,
}

impl CocoGitto {
    pub fn create_monorepo_version(&mut self, opts: BumpOptions) -> Result<()> {
        if opts.increment == IncrementCommand::Auto || opts.include_packages {
            if SETTINGS.generate_mono_repository_global_tag {
                self.create_monorepo_version_auto(opts)
            } else {
                if opts.annotated.is_some() {
                    warn!(
                        "--annotated flag is not supported for package bumps without a global tag"
                    );
                }
                self.create_all_package_version_auto(opts)
            }
        } else {
            self.create_monorepo_version_manual(opts)
        }
    }

    pub fn create_all_package_version_auto(&mut self, opts: BumpOptions) -> Result<()> {
        self.pre_bump_checks(opts.skip_untracked)?;
        // Get package bumps
        let bumps = self.get_packages_bumps(&opts)?;

        if bumps.is_empty() {
            print!("No conventional commits found for your packages that required a bump. Changelogs will be updated on the next bump.\nPre-Hooks and Post-Hooks have been skipped.\n");
            return Ok(());
        }

        if opts.dry_run {
            for bump in bumps {
                println!("{}", bump.next)
            }
            return Ok(());
        }

        let hook_result = self.run_hooks(None::<&BumpResult>, HookType::PreBump, opts.hooks_config);

        self.repository.add_all()?;
        self.unwrap_or_stash_and_exit(&Tag::default(), hook_result);
        self.bump_packages(opts.hooks_config, &bumps)?;

        self.create_bump_commit(&opts, "bump packages")?;

        if SETTINGS.generate_mono_repository_package_tags {
            for bump in &bumps {
                self.create_bump_tag(bump, None)?;
            }
        }

        // Run per package post hooks
        for bump in bumps {
            self.run_hooks(Some(&bump), HookType::PostBump, opts.hooks_config)?;
        }

        // Run global post hooks
        self.run_hooks(None::<&BumpResult>, HookType::PostBump, opts.hooks_config)?;

        Ok(())
    }

    fn create_monorepo_version_auto(&mut self, opts: BumpOptions) -> Result<()> {
        self.pre_bump_checks(opts.skip_untracked)?;
        // Get package bumps
        let bumps = self.get_packages_bumps(&opts)?;
        if bumps.is_empty() {
            print!("No conventional commits found for your packages that required a bump. Changelogs will be updated on the next bump.\nPre-Hooks and Post-Hooks have been skipped.\n");
            return Ok(());
        }

        // Manual bump with `--include-packages` -> don't override increment command
        let increment = if opts.increment != IncrementCommand::Auto {
            opts.increment.clone()
        } else if SETTINGS.generate_mono_repository_package_tags {
            // Get the greatest package increment among public api packages
            IncrementCommand::AutoMonoRepoGlobal(
                bumps
                    .iter()
                    .filter(|bump| bump.package.1.public_api)
                    .map(|bump| bump.increment())
                    .max(),
            )
        } else {
            IncrementCommand::Auto
        };

        let bump_res = opts.get_new_version(&self.repository, false, Some(increment))?;

        if opts.dry_run {
            for bump in bumps {
                println!("{}", bump.next)
            }
            print!("{}", bump_res.next);
            return Ok(());
        }

        let mut template_context = vec![];
        for bump in &bumps {
            template_context.push(PackageBumpContext {
                package_name: bump.package.0,
                package_path: &bump.package.1.path,
                version: bump.next.clone().into(),
                from: Some(bump.current.clone())
                    .filter(|tag| !tag.is_zero())
                    .map(Into::into)
                    .or_else(|| {
                        let first = self.repository.get_cache().first;
                        Some(ReleaseVersion::new(first))
                    }),
            })
        }
        template_context.sort_by_key(|package| package.package_name);

        if !SETTINGS.disable_changelog {
            let pattern = self.get_bump_revspec(&bump_res.current);
            let changelog = self.get_monorepo_global_changelog_for_version(
                pattern,
                bump_res.current.clone().into(),
                bump_res.next.clone(),
            )?;

            changelog.pretty_print_bump_summary()?;

            let path = settings::changelog_path();
            let template = SETTINGS.get_monorepo_changelog_template()?;

            changelog.write_to_file(
                path,
                template,
                ReleaseType::MonoRepo(MonoRepoContext {
                    package_lock: false,
                    packages: template_context,
                }),
            )?;
        }

        let hook_result = self.run_hooks(Some(&bump_res), HookType::PreBump, opts.hooks_config);

        self.repository.add_all()?;
        self.unwrap_or_stash_and_exit(&bump_res.next, hook_result);
        self.bump_packages(opts.hooks_config, &bumps)?;

        self.create_bump_commit(&opts, &bump_res.next)?;

        if SETTINGS.generate_mono_repository_package_tags {
            for bump in &bumps {
                self.create_bump_tag(bump, None)?;
            }
        }

        self.create_bump_tag(&bump_res, opts.annotated.as_deref())?;

        // Run per package post hooks
        for bump in bumps {
            self.run_hooks(Some(&bump), HookType::PostBump, opts.hooks_config)?;
        }

        // Run global post hooks
        self.run_hooks(Some(&bump_res), HookType::PostBump, opts.hooks_config)?;

        Ok(())
    }

    fn create_monorepo_version_manual(&mut self, opts: BumpOptions) -> Result<()> {
        self.pre_bump_checks(opts.skip_untracked)?;
        // Get package bumps
        let bumps = self.get_current_packages()?;

        let bump_res = opts.get_new_version(&self.repository, false, None)?;

        if opts.dry_run {
            print!("{}", bump_res.next);
            return Ok(());
        }

        let mut template_context = vec![];
        for bump in &bumps {
            template_context.push(PackageBumpContext {
                package_name: bump.package_name,
                package_path: bump.package_path,
                version: bump.version.clone().into(),
                from: None,
            })
        }
        template_context.sort_by_key(|package| package.package_name);

        if !SETTINGS.disable_changelog {
            let pattern = self.get_bump_revspec(&bump_res.current);
            let changelog = self.get_monorepo_global_changelog_for_version(
                pattern,
                bump_res.current.clone().into(),
                bump_res.next.clone(),
            )?;

            changelog.pretty_print_bump_summary()?;

            let path = settings::changelog_path();
            let template = SETTINGS.get_monorepo_changelog_template()?;

            changelog.write_to_file(
                path,
                template,
                ReleaseType::MonoRepo(MonoRepoContext {
                    package_lock: true,
                    packages: template_context,
                }),
            )?;
        }

        let hook_result = self.run_hooks(Some(&bump_res), HookType::PreBump, opts.hooks_config);

        self.repository.add_all()?;
        self.unwrap_or_stash_and_exit(&Tag::default(), hook_result);

        self.create_bump_commit(&opts, &bump_res.next)?;

        self.create_bump_tag(&bump_res, opts.annotated.as_deref())?;

        // Run global post hooks
        self.run_hooks(Some(&bump_res), HookType::PostBump, opts.hooks_config)?;

        Ok(())
    }

    pub fn get_current_packages(&self) -> Result<Vec<PackageData<'static>>> {
        let mut packages = vec![];
        for (package_name, package) in SETTINGS.packages.iter() {
            let tag = match self.repository.get_latest_tag(Some(package_name), false) {
                Ok(tag) => tag,
                Err(TagError::NoTag) => Tag::default(),
                Err(other) => bail!(other),
            };
            packages.push(PackageData {
                package_name,
                package_path: &package.path,
                version: tag,
            })
        }

        Ok(packages)
    }

    // Calculate all package bump
    fn get_packages_bumps(&self, opts: &BumpOptions) -> Result<Vec<BumpResult<WithPkg<'static>>>> {
        let mut package_bumps = vec![];
        let mut packages: Vec<(&String, &MonoRepoPackage)> = SETTINGS.packages.iter().collect();
        packages.sort_by(|a, b| a.1.bump_order.cmp(&b.1.bump_order));

        for (package_name, package) in packages {
            let increment = if opts.increment != IncrementCommand::Auto {
                opts.increment.clone()
            } else {
                IncrementCommand::AutoPackage(package_name.to_string())
            };
            let opts = opts.clone().set_package((package_name.as_str(), package));

            let bump_res = opts.get_new_version(&self.repository, true, Some(increment))?;
            if bump_res.no_change() || !bump_res.had_commits {
                continue;
            }

            package_bumps.push(bump_res);
        }

        Ok(package_bumps)
    }

    // Run pre hooks and generate changelog for each package and git add the generated content
    fn bump_packages(
        &mut self,
        hooks_config: Option<&str>,
        package_bumps: &Vec<BumpResult<WithPkg>>,
    ) -> Result<()> {
        for bump in package_bumps {
            if !SETTINGS.disable_changelog {
                let pattern = self.get_bump_revspec(&bump.current);
                let changelog = self.get_package_changelog_with_target_version(
                    pattern,
                    bump.next.clone(),
                    bump.package.0,
                )?;

                changelog.pretty_print_bump_summary()?;

                let path = bump.package.1.changelog_path();
                let template = SETTINGS.get_package_changelog_template()?;

                let additional_context = ReleaseType::Package(PackageContext {
                    package_name: bump.package.0,
                });

                changelog.write_to_file(&path, template, additional_context)?;
                info!("\tChangelog updated {:?}", path);
            }

            let hook_result = self.run_hooks(Some(bump), HookType::PreBump, hooks_config);

            self.repository.add_all()?;
            self.unwrap_or_stash_and_exit(&bump.next, hook_result);
        }

        Ok(())
    }
}
