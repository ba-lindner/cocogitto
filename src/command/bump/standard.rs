use crate::command::bump::BumpOptions;

use crate::conventional::changelog::ReleaseType;

use crate::conventional::version::Increment;
use crate::git::rev::filters::CommitRangeFilter;
use crate::git::tag::Tag;
use crate::settings::HookType;
use crate::{settings, CocoGitto, SETTINGS};
use anyhow::Result;
use colored::*;
use log::info;
use tera::Tera;

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

        let hook_result = self.run_hooks(HookType::PreBump, &bump, opts.hooks_config);

        self.repository.add_all()?;
        self.unwrap_or_stash_and_exit(&Tag::default(), hook_result);

        let disable_bump_commit = opts.disable_bump_commit || SETTINGS.disable_bump_commit;

        if !disable_bump_commit {
            let sign = self.repository.gpg_sign();

            if opts.skip_ci || opts.skip_ci_override.is_some() {
                let skip_ci_pattern = opts.skip_ci_override.unwrap_or(SETTINGS.skip_ci.clone());
                self.repository.commit(
                    &format!("chore(version): {} {}", bump.next, skip_ci_pattern),
                    sign,
                    true,
                )?;
            } else {
                self.repository
                    .commit(&format!("chore(version): {}", bump.next), sign, true)?;
            }
        }

        if let Some(msg_tmpl) = opts.annotated {
            let mut context = tera::Context::new();
            context.insert("latest", &bump.current.version.to_string());
            context.insert("version", &bump.next.version.to_string());
            let msg = Tera::one_off(&msg_tmpl, &context, false)?;
            self.repository
                .create_annotated_tag(&bump.next, &msg, disable_bump_commit)?;
        } else {
            self.repository
                .create_tag(&bump.next, disable_bump_commit)?;
        }

        self.run_hooks(HookType::PostBump, &bump, opts.hooks_config)?;

        let bump = format!("{} -> {}", bump.current, bump.next).green();
        info!("Bumped version: {}", bump);

        Ok(())
    }
}
