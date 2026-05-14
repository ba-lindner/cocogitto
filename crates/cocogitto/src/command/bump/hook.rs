use anyhow::{Context as _, Result};
use colored::Colorize;
use itertools::Itertools;
use log::{error, info};

use super::BumpResult;
use crate::{
    error::BumpError,
    hook::{Hook, HookVersion, Hooks},
    settings::{HookType, Settings},
    target::Target,
    CocoGitto,
};

impl CocoGitto {
    pub(super) fn run_hooks(
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

            std::process::exit(1);
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
