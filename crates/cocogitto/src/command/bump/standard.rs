use crate::command::bump::BumpOptions;

use crate::conventional::changelog::ReleaseType;

use crate::settings::HookType;
use crate::target::Target;
use crate::{settings, CocoGitto, SETTINGS};
use anyhow::Result;
use colored::*;
use log::info;

impl CocoGitto {
    pub fn create_version(&mut self, opts: BumpOptions) -> Result<()> {
        self.pre_bump_checks(opts.skip_untracked)?;

        let target = Target::Standard;

        let bump_res = self.get_new_version(&opts, None, false, None)?;
        if bump_res.no_change() {
            print!("No conventional commits for your repository that required a bump. Changelogs will be updated on the next bump.\nPre-Hooks and Post-Hooks have been skipped.\n");
            return Ok(());
        }

        if opts.dry_run {
            print!("{}", bump_res.next);
            return Ok(());
        }

        let pattern = self.get_bump_revspec(&bump_res.current);

        if !SETTINGS.disable_changelog {
            let changelog =
                self.get_changelog_with_target_version(pattern, target, bump_res.next.clone())?;
            changelog.pretty_print_bump_summary()?;

            let path = settings::changelog_path();
            let template = SETTINGS.get_changelog_template()?;

            changelog.write_to_file(path, template, ReleaseType::Standard)?;
        }

        self.run_hooks(
            Some(&bump_res),
            target,
            HookType::PreBump,
            opts.hooks_config,
        )?;

        self.create_bump_commit(&opts, &bump_res.next)?;

        self.create_bump_tag(&bump_res, opts.annotated.as_deref())?;

        self.run_hooks(
            Some(&bump_res),
            target,
            HookType::PostBump,
            opts.hooks_config,
        )?;

        let bump = format!("{} -> {}", bump_res.current, bump_res.next).green();
        info!("Bumped version: {}", bump);

        Ok(())
    }
}
