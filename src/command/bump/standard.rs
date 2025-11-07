use crate::command::bump::BumpOptions;

use crate::conventional::changelog::ReleaseType;

use crate::git::tag::Tag;
use crate::hook::HookVersion;
use crate::settings::HookType;
use crate::{settings, CocoGitto, SETTINGS};
use anyhow::Result;
use colored::*;
use log::info;
use tera::Tera;

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

        let current = self
            .repository
            .get_latest_tag(None, false)
            .map(HookVersion::new)
            .ok();

        let next_version = HookVersion::new(bump_res.next.clone());

        let hook_result =
            self.run_hooks(bump_res.as_hook_options(HookType::PreBump, opts.hooks_config));

        self.repository.add_all()?;
        self.unwrap_or_stash_and_exit(&Tag::default(), hook_result);

        let disable_bump_commit = opts.disable_bump_commit || SETTINGS.disable_bump_commit;

        if !disable_bump_commit {
            let sign = self.repository.gpg_sign();

            if opts.skip_ci || opts.skip_ci_override.is_some() {
                let skip_ci_pattern = opts.skip_ci_override.unwrap_or(SETTINGS.skip_ci.clone());
                self.repository.commit(
                    &format!(
                        "chore(version): {} {}",
                        next_version.prefixed_tag, skip_ci_pattern
                    ),
                    sign,
                    true,
                )?;
            } else {
                self.repository.commit(
                    &format!("chore(version): {}", next_version.prefixed_tag),
                    sign,
                    true,
                )?;
            }
        }

        if let Some(msg_tmpl) = opts.annotated {
            let mut context = tera::Context::new();
            context.insert("latest", &bump_res.current.version.to_string());
            context.insert("version", &bump_res.next.version.to_string());
            let msg = Tera::one_off(&msg_tmpl, &context, false)?;
            self.repository
                .create_annotated_tag(&bump_res.next, &msg, disable_bump_commit)?;
        } else {
            self.repository
                .create_tag(&bump_res.next, disable_bump_commit)?;
        }

        self.run_hooks(bump_res.as_hook_options(HookType::PostBump, opts.hooks_config))?;

        let current = current
            .map(|current| current.prefixed_tag.to_string())
            .unwrap_or_else(|| "...".to_string());
        let bump = format!("{} -> {}", current, next_version.prefixed_tag).green();
        info!("Bumped version: {}", bump);

        Ok(())
    }
}
