use crate::command::bump::{BumpOptions, HookRunOptions};

use crate::conventional::changelog::ReleaseType;

use crate::conventional::changelog::context::PackageContext;
use crate::git::tag::Tag;
use crate::hook::HookVersion;
use crate::target::Target;
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

        let tag = Tag::create(
            bump_res.next.version,
            opts.target.as_package().map(ToString::to_string),
        );

        if opts.dry_run {
            print!("{tag}");
            return Ok(());
        }

        let pattern = self.get_bump_revspec(&bump_res.current);

        if !SETTINGS.disable_changelog {
            let changelog =
                self.get_changelog_with_target_version(pattern, opts.target, tag.clone())?;
            changelog.pretty_print_bump_summary()?;

            let (path, template, kind) = match opts.target {
                Target::Standard => (
                    settings::changelog_path().clone(),
                    SETTINGS.get_changelog_template()?,
                    ReleaseType::Standard,
                ),
                Target::Package { name, package } => (
                    package.changelog_path(),
                    SETTINGS.get_package_changelog_template()?,
                    ReleaseType::Package(PackageContext { package_name: name }),
                ),
                _ => unreachable!(),
            };
            changelog.write_to_file(path, template, kind)?;
        }

        let current = self
            .repository
            .get_latest_tag(opts.target.as_package(), true)
            .map(HookVersion::new)
            .ok();

        let next_version = HookVersion::new(tag.clone());

        // !PACKAGE
        let hook_result = self.run_hooks(
            HookRunOptions::pre_bump()
                .current_tag(current.as_ref())
                .next_version(&next_version)
                .hook_profile(opts.hooks_config),
        );

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
            context.insert("version", &tag.version.to_string());
            let msg = Tera::one_off(&msg_tmpl, &context, false)?;
            self.repository
                .create_annotated_tag(&tag, &msg, disable_bump_commit)?;
        } else {
            self.repository.create_tag(&tag, disable_bump_commit)?;
        }

        // !PACKAGE
        self.run_hooks(
            HookRunOptions::post_bump()
                .current_tag(current.as_ref())
                .next_version(&next_version)
                .hook_profile(opts.hooks_config),
        )?;

        let current = current
            .map(|current| current.prefixed_tag.to_string())
            .unwrap_or_else(|| "...".to_string());
        let bump = format!("{} -> {}", current, next_version.prefixed_tag).green();
        // !PACKAGE: info!("Bumped package {} version: {}", opts.package_name, bump);
        info!("Bumped version: {}", bump);

        Ok(())
    }
}
