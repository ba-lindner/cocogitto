use crate::command::bump::{BumpOptions, WithPkg};
use crate::conventional::changelog::template::Empty;
use crate::conventional::version::Increment;
use crate::git::rev::filters::CommitRangeFilter;
use crate::settings::HookType;
use crate::CocoGitto;
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

        self.render_changelog(&bump, Empty)?;

        self.run_hooks(HookType::PreBump, &bump, opts.hooks_config)?;

        self.create_bump_commit(&bump, &opts)?;
        self.create_bump_tag(&bump, &opts)?;

        self.run_hooks(HookType::PostBump, &bump, opts.hooks_config)?;

        let bump = format!("{} -> {}", bump.current, bump.next).green();
        info!("Bumped package {} version: {}", opts.package.0, bump);

        Ok(())
    }
}
