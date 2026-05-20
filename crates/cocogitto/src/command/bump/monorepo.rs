use std::collections::HashMap;

use crate::command::bump::{BumpOptions, BumpResult};
use crate::conventional::changelog::context::PackageBumpContext;
use crate::conventional::version::Increment;
use crate::settings::{HookType, MonoRepoPackage};
use crate::target::Target;
use crate::{CocoGitto, SETTINGS};
use anyhow::Result;

use log::warn;

use crate::git::oid::ReleaseVersion;

#[derive(Debug)]
struct PackageBumpData {
    name: &'static str,
    package: &'static MonoRepoPackage,
    res: BumpResult,
}

impl CocoGitto {
    pub(super) fn create_monorepo_version(&mut self, opts: BumpOptions) -> Result<()> {
        self.pre_bump_checks(opts.skip_untracked)?;
        if !SETTINGS.generate_mono_repository_global_tag && opts.annotated.is_some() {
            warn!("--annotated flag is not supported for package bumps without a global tag");
        }

        // Get package bumps
        let bumps = self.get_packages_bumps(&opts)?;
        if bumps.is_empty() {
            print!("No conventional commits found for your packages that required a bump. Changelogs will be updated on the next bump.\nPre-Hooks and Post-Hooks have been skipped.\n");
            return Ok(());
        }

        let bump_res = if SETTINGS.generate_mono_repository_global_tag {
            let package_increment = bumps
                .iter()
                .filter(|bump| {
                    // required to reproduce current behaviour
                    bump.package.public_api || !SETTINGS.generate_mono_repository_package_tags
                })
                .map(|bump| bump.res.increment())
                .max();
            Some(self.get_new_version(&opts, false, package_increment)?)
        } else {
            None
        };

        if opts.dry_run {
            for bump in bumps {
                println!("{}", bump.res.next)
            }
            if let Some(bump_res) = bump_res {
                print!("{}", bump_res.next);
            }
            return Ok(());
        }

        if let Some(bump_res) = &bump_res {
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

            self.render_changelog(bump_res, Some(template_context))?;
        }

        self.run_hooks(bump_res.as_ref(), HookType::PreBump, opts.hooks_config)?;
        for bump in &bumps {
            self.render_changelog(&bump.res, None)?;

            self.run_hooks(Some(&bump.res), HookType::PreBump, opts.hooks_config)?;
        }

        self.create_bump_commit(
            &opts,
            bump_res
                .as_ref()
                .map(|bump| bump.next.to_string())
                .unwrap_or("bump packages".to_string()),
        )?;

        if SETTINGS.generate_mono_repository_package_tags {
            for bump in &bumps {
                self.create_bump_tag(&bump.res, None)?;
            }
        }

        if let Some(bump_res) = &bump_res {
            self.create_bump_tag(bump_res, opts.annotated.as_deref())?;
        }

        // Run per package post hooks
        for bump in bumps {
            self.run_hooks(Some(&bump.res), HookType::PostBump, opts.hooks_config)?;
        }

        // Run global post hooks
        self.run_hooks(bump_res.as_ref(), HookType::PostBump, opts.hooks_config)?;

        Ok(())
    }

    fn get_packages_bumps(&self, opts: &BumpOptions) -> Result<Vec<PackageBumpData>> {
        let mut package_bumps = vec![];
        let mut packages: Vec<(&String, &MonoRepoPackage)> = SETTINGS
            .monorepo
            .as_ref()
            .map(|m| m.packages.iter().collect())
            .unwrap_or_default();

        if let Some(monorepo) = &SETTINGS.monorepo {
            if monorepo.resolver.is_some() {
                use cocogitto_dependency_resolver::DepGraphResolver;

                let resolver_name = monorepo.resolver.as_deref().unwrap_or("Cargo");

                let resolver = match resolver_name {
                    "Cargo" => DepGraphResolver::Cargo,
                    "Maven" => DepGraphResolver::Maven,
                    "Npm" => DepGraphResolver::Npm,
                    _ => DepGraphResolver::Cargo, // Default fallback
                };

                let manifest_path = self.repository.get_repo_dir().and_then(|repo_path| {
                    // Try common manifest files
                    let cargo_toml = repo_path.join("Cargo.toml");
                    if cargo_toml.exists() {
                        return Some(cargo_toml);
                    }

                    let pom_xml = repo_path.join("pom.xml");
                    if pom_xml.exists() {
                        return Some(pom_xml);
                    }

                    let package_json = repo_path.join("package.json");
                    if package_json.exists() {
                        return Some(package_json);
                    }

                    None
                });

                if let Some(manifest_path) = manifest_path {
                    let dependencies =
                        resolver.topological_sort(manifest_path.to_str().unwrap_or(""));

                    let order_map: HashMap<&str, usize> = dependencies
                        .iter()
                        .enumerate()
                        .map(|(i, name)| (name.as_str(), i))
                        .collect();

                    packages
                        .sort_by_key(|pkg| order_map.get(pkg.0.as_str()).unwrap_or(&usize::MAX));
                }
            }
        } else {
            packages.sort_by_key(|pkg| pkg.1.bump_order);
        }

        for (name, package) in packages {
            let opts = BumpOptions {
                target: Target::Package { name, package },
                ..opts.clone()
            };

            let bump_res = self.get_new_version(&opts, true, None)?;
            if bump_res.increment() != Increment::NoBump {
                package_bumps.push(PackageBumpData {
                    res: bump_res,
                    name,
                    package,
                })
            }
        }

        Ok(package_bumps)
    }
}
