use crate::command::bump::BumpOptions;

use crate::conventional::changelog::ReleaseType;

use crate::conventional::version::Increment;
use crate::git::rev::filters::CommitRangeFilter;
use crate::settings::HookType;
use crate::{settings, CocoGitto, SETTINGS};
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

        if !SETTINGS.disable_changelog {
            bump.release.pretty_print_bump_summary()?;

            let path = settings::changelog_path();
            let template = SETTINGS.get_changelog_template()?;

            bump.release
                .write_to_file(path, template, ReleaseType::Standard)?;
        }

        self.run_hooks(HookType::PreBump, &bump, opts.hooks_config)?;

        self.create_bump_commit(&bump, &opts)?;
        self.create_bump_tag(&bump, &opts)?;

        self.run_hooks(HookType::PostBump, &bump, opts.hooks_config)?;

        let bump = format!("{} -> {}", bump.current, bump.next).green();
        info!("Bumped version: {}", bump);

        Ok(())
    }
}
