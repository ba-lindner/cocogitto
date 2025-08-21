use crate::command::bump::{BumpOptions, WithPkg};
use crate::conventional::changelog::template::PackageContext;
use crate::conventional::changelog::ReleaseType;
use crate::conventional::version::Increment;
use crate::git::rev::filters::CommitRangeFilter;
use crate::settings::HookType;
use crate::{CocoGitto, SETTINGS};
use anyhow::Result;
use colored::*;
use log::info;

impl CocoGitto {
    pub fn create_package_version(&mut self, opts: BumpOptions<WithPkg>) -> Result<()> {
        self.pre_bump_checks(opts.skip_untracked)?;

        let bump = self.get_next_version(
            &opts,
            CommitRangeFilter::Package(opts.package.0),
            None,
            false,
        )?;
        if bump.increment() == Increment::NoBump {
            print!("No conventional commits found for {} that required a bump. Changelog will be updated on the next bump.\nPre-Hooks and Post-Hooks have been skipped.\n", opts.package.0);
            return Ok(());
        }

        if opts.dry_run {
            print!("{}", bump.next);
            return Ok(());
        }

        if !SETTINGS.disable_changelog {
            bump.release.pretty_print_bump_summary()?;

            let path = opts.package.1.changelog_path();
            let template = SETTINGS.get_package_changelog_template()?;
            let additional_context = ReleaseType::Package(PackageContext {
                package_name: opts.package.0,
            });
            bump.release
                .write_to_file(path, template, additional_context)?;
        }

        self.run_hooks(HookType::PreBump, &bump, opts.hooks_config)?;

        self.create_bump_commit(&bump, &opts)?;
        self.create_bump_tag(&bump, &opts)?;

        self.run_hooks(HookType::PostBump, &bump, opts.hooks_config)?;

        let bump = format!("{} -> {}", bump.current, bump.next).green();
        info!("Bumped package {} version: {}", opts.package.0, bump);

        Ok(())
    }
}
