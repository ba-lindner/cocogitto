use crate::command::bump::PackageBumpOptions;
use crate::conventional::changelog::context::PackageContext;
use crate::conventional::changelog::ReleaseType;
use crate::settings::HookType;
use crate::target::Target;
use crate::{CocoGitto, SETTINGS};
use anyhow::Result;
use colored::*;
use log::info;
use tera::Tera;

impl CocoGitto {
    pub fn create_package_version(&mut self, opts: PackageBumpOptions) -> Result<()> {
        self.pre_bump_checks(opts.skip_untracked)?;

        let target = Target::package(opts.package_name);

        let bump_res = opts.common().get_new_version(
            &self.repository,
            Some(opts.package_name),
            false,
            None,
        )?;
        if bump_res.no_change() {
            print!("No conventional commits found for {} that required a bump. Changelog will be updated on the next bump.\nPre-Hooks and Post-Hooks have been skipped.\n", opts.package_name);
            return Ok(());
        }

        if opts.dry_run {
            print!("{}", bump_res.next);
            return Ok(());
        }

        if !SETTINGS.disable_changelog {
            let pattern = self.get_bump_revspec(&bump_res.current);
            let changelog =
                self.get_changelog_with_target_version(pattern, target, bump_res.next.clone())?;

            changelog.pretty_print_bump_summary()?;

            let path = opts.package.changelog_path();
            let template = SETTINGS.get_package_changelog_template()?;
            let additional_context = ReleaseType::Package(PackageContext {
                package_name: opts.package_name,
            });
            changelog.write_to_file(path, template, additional_context)?;
        }

        self.run_hooks(
            Some(&bump_res),
            target,
            HookType::PreBump,
            opts.hooks_config,
        )?;

        let disable_bump_commit = opts.disable_bump_commit || SETTINGS.disable_bump_commit;

        if !disable_bump_commit {
            let sign = self.repository.gpg_sign();

            if opts.skip_ci || opts.skip_ci_override.is_some() {
                let skip_ci_pattern = opts.skip_ci_override.unwrap_or(SETTINGS.skip_ci.clone());
                self.repository.commit(
                    &format!("chore(version): {} {skip_ci_pattern}", bump_res.next),
                    sign,
                    true,
                )?;
            } else {
                self.repository.commit(
                    &format!("chore(version): {}", bump_res.next),
                    sign,
                    true,
                )?;
            }
        }

        if let Some(msg_tmpl) = opts.annotated {
            let mut context = tera::Context::new();
            context.insert("latest", &bump_res.current.version.to_string());
            context.insert("version", &bump_res.next.version.to_string());
            let msg = Tera::one_off(&msg_tmpl, &context, false)?;
            self.repository
                .create_annotated_tag(&bump_res.next, &msg, disable_bump_commit)?;
        } else {
            self.repository
                .create_tag(&bump_res.next, disable_bump_commit)?;
        }

        self.run_hooks(
            Some(&bump_res),
            target,
            HookType::PostBump,
            opts.hooks_config,
        )?;

        let bump = format!("{} -> {}", bump_res.current, bump_res.next).green();
        info!("Bumped package {} version: {}", opts.package_name, bump);

        Ok(())
    }
}
