use crate::command::bump::BumpOptions;

use crate::conventional::changelog::ReleaseType;

use crate::git::tag::Tag;
use crate::settings::HookType;
use crate::{settings, CocoGitto, SETTINGS};
use anyhow::Result;
use colored::*;
use log::info;

impl CocoGitto {
    pub fn create_version(&mut self, opts: BumpOptions) -> Result<()> {
        self.pre_bump_checks(opts.skip_untracked)?;

        let bump_res = opts.get_new_version(&self.repository, false, None)?;
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
                self.get_changelog_with_target_version(pattern, bump_res.next.clone())?;
            changelog.pretty_print_bump_summary()?;

            let path = settings::changelog_path();
            let template = SETTINGS.get_changelog_template()?;

            changelog.write_to_file(path, template, ReleaseType::Standard)?;
        }

        let hook_result = self.run_hooks(Some(&bump_res), HookType::PreBump, opts.hooks_config);

        self.repository.add_all()?;
        self.unwrap_or_stash_and_exit(&Tag::default(), hook_result);

        self.create_bump_commit(&opts, &bump_res.next)?;

        self.create_bump_tag(&bump_res, opts.annotated.as_deref())?;

        self.run_hooks(Some(&bump_res), HookType::PostBump, opts.hooks_config)?;

        let bump = format!("{} -> {}", bump_res.current, bump_res.next).green();
        info!("Bumped version: {}", bump);

        Ok(())
    }
}
