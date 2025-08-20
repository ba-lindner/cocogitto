use crate::command::bump::PackageBumpOptions;
use crate::conventional::changelog::template::PackageContext;
use crate::conventional::changelog::ReleaseType;
use crate::conventional::version::Increment;
use crate::git::rev::filters::CommitRangeFilter;
use crate::git::tag::Tag;
use crate::settings::HookType;
use crate::{CocoGitto, SETTINGS};
use anyhow::Result;
use colored::*;
use log::info;
use tera::Tera;

impl CocoGitto {
    pub fn create_package_version(&mut self, opts: PackageBumpOptions) -> Result<()> {
        self.pre_bump_checks(opts.skip_untracked)?;

        let bump = self.get_next_version(
            &opts.common(),
            CommitRangeFilter::Package(opts.package_name),
            None,
            false,
        )?;
        if bump.increment() == Increment::NoBump {
            print!("No conventional commits found for {} that required a bump. Changelog will be updated on the next bump.\nPre-Hooks and Post-Hooks have been skipped.\n", opts.package_name);
            return Ok(());
        }

        if opts.dry_run {
            print!("{}", bump.next);
            return Ok(());
        }

        if !SETTINGS.disable_changelog {
            bump.release.pretty_print_bump_summary()?;

            let path = opts.package.changelog_path();
            let template = SETTINGS.get_package_changelog_template()?;
            let additional_context = ReleaseType::Package(PackageContext {
                package_name: opts.package_name,
            });
            bump.release
                .write_to_file(path, template, additional_context)?;
        }

        let hook_result = self.run_hooks(
            HookType::PreBump,
            &bump,
            opts.hooks_config,
            Some((opts.package_name, opts.package)),
        );

        self.repository.add_all()?;
        self.unwrap_or_stash_and_exit(&Tag::default(), hook_result);

        let disable_bump_commit = opts.disable_bump_commit || SETTINGS.disable_bump_commit;

        if !disable_bump_commit {
            let sign = self.repository.gpg_sign();

            if opts.skip_ci || opts.skip_ci_override.is_some() {
                let skip_ci_pattern = opts.skip_ci_override.unwrap_or(SETTINGS.skip_ci.clone());
                self.repository.commit(
                    &format!("chore(version): {} {skip_ci_pattern}", bump.next),
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

        self.run_hooks(
            HookType::PostBump,
            &bump,
            opts.hooks_config,
            Some((opts.package_name, opts.package)),
        )?;

        let bump = format!("{} -> {}", bump.current, bump.next).green();
        info!("Bumped package {} version: {}", opts.package_name, bump);

        Ok(())
    }
}
