use crate::command::bump::BumpOptions;

use crate::conventional::version::Increment;
use crate::git::rev::filters::CommitRangeFilter;
use crate::settings::HookType;
use crate::CocoGitto;
use anyhow::Result;
use colored::*;
use log::info;

impl CocoGitto {
    pub fn create_version(&mut self, opts: BumpOptions) -> Result<()> {
        self.pre_bump_checks(opts.skip_untracked)?;

        let bump = self.get_next_version(&opts, CommitRangeFilter::None, None, false)?;
        if bump.increment() == Increment::NoBump {
            print!("No conventional commits for your repository that required a bump. Changelogs will be updated on the next bump.\nPre-Hooks and Post-Hooks have been skipped.\n");
            return Ok(());
        }

        if opts.dry_run {
            print!("{}", bump.next);
            return Ok(());
        }

        self.render_changelog(&bump, ())?;

        self.run_hooks(HookType::PreBump, &bump, opts.hooks_config)?;

        self.create_bump_commit(&bump, &opts)?;
        self.create_bump_tag(&bump, &opts)?;

        self.run_hooks(HookType::PostBump, &bump, opts.hooks_config)?;

        let bump = format!("{} -> {}", bump.current, bump.next).green();
        info!("Bumped version: {}", bump);

        Ok(())
    }
}
