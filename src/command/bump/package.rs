use crate::command::bump::{BumpOptions, WithPkg};
use crate::conventional::changelog::template::PackageContext;
use crate::conventional::changelog::ReleaseType;
use crate::git::tag::Tag;
use crate::settings::HookType;
use crate::{CocoGitto, SETTINGS};
use anyhow::Result;
use colored::*;
use log::info;

impl CocoGitto {
    pub fn create_package_version(&mut self, opts: BumpOptions<WithPkg>) -> Result<()> {
        self.pre_bump_checks(opts.skip_untracked)?;

        let bump_res = opts.get_new_version(&self.repository, false, None)?;
        if bump_res.no_change() {
            print!("No conventional commits found for {} that required a bump. Changelog will be updated on the next bump.\nPre-Hooks and Post-Hooks have been skipped.\n", opts.package.0);
            return Ok(());
        }

        if opts.dry_run {
            print!("{}", bump_res.next);
            return Ok(());
        }

        if !SETTINGS.disable_changelog {
            let pattern = self.get_bump_revspec(&bump_res.current);
            let changelog = self.get_package_changelog_with_target_version(
                pattern,
                bump_res.next.clone(),
                opts.package.0,
            )?;

            changelog.pretty_print_bump_summary()?;

            let path = opts.package.1.changelog_path();
            let template = SETTINGS.get_package_changelog_template()?;
            let additional_context = ReleaseType::Package(PackageContext {
                package_name: opts.package.0,
            });
            changelog.write_to_file(path, template, additional_context)?;
        }

        let hook_result = self.run_hooks(Some(&bump_res), HookType::PreBump, opts.hooks_config);

        self.repository.add_all()?;
        self.unwrap_or_stash_and_exit(&Tag::default(), hook_result);

        self.create_bump_commit(&opts, &bump_res.next)?;

        self.create_bump_tag(&bump_res, opts.annotated.as_deref())?;

        self.run_hooks(Some(&bump_res), HookType::PostBump, opts.hooks_config)?;

        let bump = format!("{} -> {}", bump_res.current, bump_res.next).green();
        info!("Bumped package {} version: {}", opts.package.0, bump);

        Ok(())
    }
}
