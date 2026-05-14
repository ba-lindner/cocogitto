use std::collections::HashMap;

use crate::command::bump::{BumpOptions, BumpResult};
use crate::conventional::changelog::context::{
    MonoRepoContext, PackageBumpContext, PackageContext,
};
use crate::conventional::changelog::ReleaseType;
use crate::conventional::version::{Increment, IncrementCommand};
use crate::git::error::TagError;
use crate::git::tag::Tag;
use crate::settings::{HookType, MonoRepoPackage};
use crate::target::Target;
use crate::{settings, CocoGitto, SETTINGS};
use anyhow::{bail, Result};

use log::{info, warn};

use crate::git::oid::ReleaseVersion;

#[derive(Debug)]
struct PackageBumpData {
    name: &'static str,
    package: &'static MonoRepoPackage,
    res: BumpResult,
}

#[derive(Debug)]
pub struct PackageData {
    pub package_name: String,
    pub package_path: String,
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

        let target = Target::Monorepo { manual: false };

        // Get package bumps
        let bumps = self.get_packages_bumps(&opts)?;

        if bumps.is_empty() {
            print!("No conventional commits found for your packages that required a bump. Changelogs will be updated on the next bump.\nPre-Hooks and Post-Hooks have been skipped.\n");
            return Ok(());
        }

        if opts.dry_run {
            for bump in bumps {
                println!("{}", bump.res.next)
            }
            return Ok(());
        }

        self.run_hooks(None, target, HookType::PreBump, opts.hooks_config)?;

        self.bump_packages(opts.hooks_config, &bumps)?;

        self.create_bump_commit(&opts, "bump packages")?;

        if SETTINGS.generate_mono_repository_package_tags {
            for bump in &bumps {
                self.create_bump_tag(&bump.res, None)?;
            }
        }

        // Run per package post hooks
        for bump in bumps {
            self.run_hooks(
                Some(&bump.res),
                Target::Package {
                    name: bump.name,
                    package: bump.package,
                },
                HookType::PostBump,
                opts.hooks_config,
            )?;
        }

        // Run global post hooks
        self.run_hooks(None, target, HookType::PostBump, opts.hooks_config)?;

        Ok(())
    }

    fn create_monorepo_version_auto(&mut self, opts: BumpOptions) -> Result<()> {
        self.pre_bump_checks(opts.skip_untracked)?;

        let target = Target::Monorepo { manual: false };

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
                    .filter(|bump| bump.package.public_api)
                    .map(|bump| {
                        bump.res
                            .next
                            .get_increment_from(&bump.res.current)
                            .unwrap_or(Increment::NoBump)
                    })
                    .max(),
            )
        } else {
            IncrementCommand::Auto
        };

        let bump_res = self.get_new_version(&opts, None, false, Some(increment))?;

        if opts.dry_run {
            for bump in bumps {
                println!("{}", bump.res.next)
            }
            print!("{}", bump_res.next);
            return Ok(());
        }

        let mut template_context = vec![];
        for bump in &bumps {
            let from = if bump.res.current.is_zero() {
                let first = self
                    .repository
                    .get_first_commit()
                    .expect("non empty repository");
                ReleaseVersion::new(first)
            } else {
                bump.res.current.clone().into()
            };
            template_context.push(PackageBumpContext {
                package_name: bump.name,
                package_path: bump.package.path.to_string_lossy().to_string(),
                version: bump.res.next.clone().into(),
                from: Some(from),
            })
        }
        template_context.sort_by_key(|package| package.package_name);

        if !SETTINGS.disable_changelog {
            let pattern = self.get_bump_revspec(&bump_res.current);
            let changelog =
                self.get_changelog_with_target_version(pattern, target, bump_res.next.clone())?;

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

        self.run_hooks(
            Some(&bump_res),
            target,
            HookType::PreBump,
            opts.hooks_config,
        )?;
        self.bump_packages(opts.hooks_config, &bumps)?;

        self.create_bump_commit(&opts, &bump_res.next)?;

        if SETTINGS.generate_mono_repository_package_tags {
            for bump in &bumps {
                self.create_bump_tag(&bump.res, None)?;
            }
        }

        self.create_bump_tag(&bump_res, opts.annotated.as_deref())?;

        // Run per package post hooks
        for bump in bumps {
            self.run_hooks(
                Some(&bump.res),
                Target::Package {
                    name: bump.name,
                    package: bump.package,
                },
                HookType::PostBump,
                opts.hooks_config,
            )?;
        }

        // Run global post hooks
        self.run_hooks(
            Some(&bump_res),
            target,
            HookType::PostBump,
            opts.hooks_config,
        )?;

        Ok(())
    }

    fn create_monorepo_version_manual(&mut self, opts: BumpOptions) -> Result<()> {
        self.pre_bump_checks(opts.skip_untracked)?;

        let target = Target::Monorepo { manual: true };

        // Get package bumps
        let bumps = self.get_current_packages()?;

        let bump_res = self.get_new_version(&opts, None, false, None)?;

        if opts.dry_run {
            print!("{}", bump_res.next);
            return Ok(());
        }

        let mut template_context = vec![];
        for bump in &bumps {
            template_context.push(PackageBumpContext {
                package_name: &bump.package_name,
                package_path: bump.package_path.clone(),
                version: bump.version.clone().into(),
                from: None,
            })
        }
        template_context.sort_by_key(|package| package.package_name);

        if !SETTINGS.disable_changelog {
            let pattern = self.get_bump_revspec(&bump_res.current);
            let changelog =
                self.get_changelog_with_target_version(pattern, target, bump_res.next.clone())?;

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

        self.run_hooks(
            Some(&bump_res),
            target,
            HookType::PreBump,
            opts.hooks_config,
        )?;

        self.create_bump_commit(&opts, &bump_res.next)?;

        self.create_bump_tag(&bump_res, opts.annotated.as_deref())?;

        // Run global post hooks
        self.run_hooks(
            Some(&bump_res),
            target,
            HookType::PostBump,
            opts.hooks_config,
        )?;

        Ok(())
    }

    pub fn get_current_packages(&self) -> Result<Vec<PackageData>> {
        let mut packages = vec![];
        for (package_name, package) in SETTINGS
            .monorepo
            .as_ref()
            .map(|m| m.packages.iter())
            .unwrap_or_default()
        {
            let tag = match self.repository.get_latest_tag(Some(package_name), false) {
                Ok(tag) => tag,
                Err(TagError::NoTag) => Tag::default(),
                Err(other) => bail!(other),
            };
            packages.push(PackageData {
                package_name: package_name.to_string(),
                package_path: package.path.to_string_lossy().to_string(),
                version: tag,
            })
        }

        Ok(packages)
    }

    fn get_package_order_map(&self) -> Option<HashMap<String, usize>> {
        use cocogitto_dependency_resolver::DepGraphResolver;

        let resolver = match SETTINGS.monorepo.as_ref()?.resolver.as_ref()?.as_str() {
            "Cargo" => DepGraphResolver::Cargo,
            "Maven" => DepGraphResolver::Maven,
            "Npm" => DepGraphResolver::Npm,
            _ => DepGraphResolver::Cargo, // Default fallback
        };

        let repo_path = self.repository.get_repo_dir()?;
        // Try common manifest files
        let manifest_path = ["Cargo.toml", "pom.xml", "package.json"]
            .iter()
            .map(|manifest| repo_path.join(manifest))
            .find(|manifest| manifest.exists())?;

        let dependencies = resolver.topological_sort(manifest_path);

        Some(
            dependencies
                .into_iter()
                .enumerate()
                .map(|(i, name)| (name, i))
                .collect(),
        )
    }

    fn get_packages_bumps(&self, opts: &BumpOptions) -> Result<Vec<PackageBumpData>> {
        let mut package_bumps = vec![];
        let mut packages: Vec<(&String, &MonoRepoPackage)> = SETTINGS
            .monorepo
            .as_ref()
            .map(|m| m.packages.iter().collect())
            .unwrap_or_default();

        let order_map = self.get_package_order_map().unwrap_or_default();
        packages.sort_by(|a, b| {
            let a_order = order_map.get(a.0).unwrap_or(&usize::MAX);
            let b_order = order_map.get(b.0).unwrap_or(&usize::MAX);
            a_order
                .cmp(b_order)
                .then(a.1.bump_order.cmp(&b.1.bump_order))
        });

        for (package_name, package) in packages {
            let increment = if opts.increment != IncrementCommand::Auto {
                opts.increment.clone()
            } else {
                IncrementCommand::AutoPackage(package_name.to_string())
            };

            let bump_res = self.get_new_version(opts, Some(package_name), true, Some(increment))?;
            if bump_res.no_change() || !bump_res.had_commits {
                continue;
            }

            let tag = Tag::create(
                bump_res.next.version.clone(),
                Some(package_name.to_string()),
            );
            let increment = tag.get_increment_from(&bump_res.current);

            if increment.is_some() {
                package_bumps.push(PackageBumpData {
                    res: bump_res,
                    name: package_name,
                    package,
                })
            }
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
            if !SETTINGS.disable_changelog {
                let pattern = self.get_bump_revspec(&bump.res.current);
                let changelog = self.get_changelog_with_target_version(
                    pattern,
                    Target::Package {
                        name: bump.name,
                        package: bump.package,
                    },
                    bump.res.next.clone(),
                )?;

                changelog.pretty_print_bump_summary()?;

                let path = bump.package.changelog_path();
                let template = SETTINGS.get_package_changelog_template()?;

                let additional_context = ReleaseType::Package(PackageContext {
                    package_name: bump.name,
                });

                changelog.write_to_file(&path, template, additional_context)?;
                info!("\tChangelog updated {:?}", path);
            }

            self.run_hooks(
                Some(&bump.res),
                Target::Package {
                    name: bump.name,
                    package: bump.package,
                },
                HookType::PreBump,
                hooks_config,
            )?;
        }

        Ok(())
    }
}
