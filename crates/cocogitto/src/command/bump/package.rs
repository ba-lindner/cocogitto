use crate::command::bump::BumpOptions;
use crate::settings::HookType;
use crate::CocoGitto;
use anyhow::Result;
use colored::*;
use log::info;

impl CocoGitto {
    pub fn create_package_version(&mut self, opts: BumpOptions) -> Result<()> {
        self.pre_bump_checks(opts.skip_untracked)?;

        let package_name = opts.target.as_package().unwrap_or_default();

        let bump_res = self.get_new_version(&opts, false, None)?;
        if bump_res.no_change() {
            print!("No conventional commits found for {package_name} that required a bump. Changelog will be updated on the next bump.\nPre-Hooks and Post-Hooks have been skipped.\n");
            return Ok(());
        }

        if opts.dry_run {
            print!("{}", bump_res.next);
            return Ok(());
        }

        self.render_changelog(&bump_res, None)?;

        self.run_hooks(Some(&bump_res), HookType::PreBump, opts.hooks_config)?;

        self.create_bump_commit(&opts, &bump_res.next)?;

        self.create_bump_tag(&bump_res, opts.annotated.as_deref())?;

        self.run_hooks(Some(&bump_res), HookType::PostBump, opts.hooks_config)?;

        let bump = format!("{} -> {}", bump_res.current, bump_res.next).green();
        info!("Bumped package {package_name} version: {bump}");

        Ok(())
    }
}
