use crate::command::bump::PackageBumpOptions;
use crate::settings::HookType;
use crate::target::Target;
use crate::CocoGitto;
use anyhow::Result;
use colored::*;
use log::info;

impl CocoGitto {
    pub fn create_package_version(&mut self, opts: PackageBumpOptions) -> Result<()> {
        self.pre_bump_checks(opts.skip_untracked)?;

        let target = Target::package(opts.package_name);

        let bump_res =
            self.get_new_version(&opts.common(), Some(opts.package_name), false, None)?;
        if bump_res.no_change() {
            print!("No conventional commits found for {} that required a bump. Changelog will be updated on the next bump.\nPre-Hooks and Post-Hooks have been skipped.\n", opts.package_name);
            return Ok(());
        }

        if opts.dry_run {
            print!("{}", bump_res.next);
            return Ok(());
        }

        self.render_changelog(&bump_res, target, None)?;

        self.run_hooks(
            Some(&bump_res),
            target,
            HookType::PreBump,
            opts.hooks_config,
        )?;

        self.create_bump_commit(&opts.common(), &bump_res.next)?;

        self.create_bump_tag(&bump_res, opts.annotated.as_deref())?;

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
