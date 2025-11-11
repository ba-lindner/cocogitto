use crate::command::bump::{BumpOptions, WithPkg};
use crate::conventional::changelog::template::PackageContext;
use crate::conventional::changelog::ReleaseType;
use crate::conventional::version::Increment;
use crate::git::tag::Tag;
use crate::settings::HookType;
use crate::CocoGitto;
use anyhow::Result;
use colored::*;
use log::info;

impl CocoGitto {
    pub fn create_package_version(&mut self, opts: BumpOptions<WithPkg>) -> Result<()> {
        self.pre_bump_checks(opts.skip_untracked)?;

        let mut bump_res = opts.get_new_version(&self.repository, false, None)?;
        if bump_res.increment() == Increment::NoBump {
            print!("No conventional commits found for {} that required a bump. Changelog will be updated on the next bump.\nPre-Hooks and Post-Hooks have been skipped.\n", opts.package.0);
            return Ok(());
        }

        if opts.dry_run {
            print!("{}", bump_res.next);
            return Ok(());
        }

        self.render_changelog(
            &mut bump_res,
            ReleaseType::Package(PackageContext {
                package_name: opts.package.0,
            }),
        )?;

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
