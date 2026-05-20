use crate::conventional::changelog::context::PackageBumpContext;
use crate::conventional::version::Increment;
use crate::conventional::version::IncrementCommand;
use crate::conventional::version::PreCommand;
use crate::git::error::TagError;
use crate::git::tag::Tag;
use crate::settings::HookType;
use crate::settings::Settings;
use crate::target::Target;
use crate::{CocoGitto, SETTINGS};
use anyhow::bail;
use anyhow::ensure;
use anyhow::Result;
use colored::Colorize;
use globset::Glob;
use log::info;
use log::warn;
use std::default::Default;

mod changelog;
mod hook;
mod monorepo;
mod version;

#[derive(Default, Clone)]
pub struct BumpOptions<'a> {
    pub target: Target,
    pub increment: IncrementCommand,
    pub pre_release: Option<PreCommand<'a>>,
    pub build: Option<&'a str>,
    pub hooks_config: Option<&'a str>,
    pub annotated: Option<String>,
    pub dry_run: bool,
    pub skip_ci: bool,
    pub skip_ci_override: Option<String>,
    pub skip_untracked: bool,
    pub disable_bump_commit: bool,
    pub include_packages: bool,
}

#[derive(Debug)]
struct BumpResult {
    target: Target,
    current: Tag,
    next: Tag,
}

impl BumpResult {
    fn no_change(&self) -> bool {
        self.current.version == self.next.version
    }

    fn increment(&self) -> Increment {
        self.next
            .get_increment_from(&self.current)
            .unwrap_or(Increment::NoBump)
    }
}

impl CocoGitto {
    pub fn create_version(&mut self, opts: BumpOptions) -> Result<()> {
        if (opts.target == Target::Monorepo { manual: false }) {
            self.create_monorepo_version(opts)
        } else {
            // standard, package and monorepo manual are all identical
            self.create_single_version(opts)
        }
    }

    fn create_single_version(&mut self, opts: BumpOptions) -> Result<()> {
        self.pre_bump_checks(opts.skip_untracked)?;

        let bump_res = self.get_new_version(&opts, false, None)?;
        if bump_res.no_change() {
            print!("No conventional commits for your repository that required a bump. Changelogs will be updated on the next bump.\nPre-Hooks and Post-Hooks have been skipped.\n");
            return Ok(());
        }

        if opts.dry_run {
            print!("{}", bump_res.next);
            return Ok(());
        }

        let mut template_context = vec![];
        for (name, package) in SETTINGS.list_packages() {
            let tag = match self.repository.get_latest_tag(Some(name), false) {
                Ok(tag) => tag,
                Err(TagError::NoTag) => Tag::default(),
                Err(other) => bail!(other),
            };
            template_context.push(PackageBumpContext {
                package_name: name,
                package_path: package.path.to_string_lossy().to_string(),
                version: tag.into(),
                from: None,
            });
        }
        self.render_changelog(&bump_res, Some(template_context))?;

        self.run_hooks(Some(&bump_res), HookType::PreBump, opts.hooks_config)?;

        self.create_bump_commit(&opts, &bump_res.next)?;

        self.create_bump_tag(&bump_res, opts.annotated.as_deref())?;

        self.run_hooks(Some(&bump_res), HookType::PostBump, opts.hooks_config)?;

        let bump = format!("{} -> {}", bump_res.current, bump_res.next).green();
        info!("Bumped version: {}", bump);

        Ok(())
    }

    fn pre_bump_checks(&mut self, skip_untracked: bool) -> Result<()> {
        if *SETTINGS == Settings::default() {
            let part1 = "Warning: using".yellow();
            let part2 = "with the default configuration. \n".yellow();
            let part3 = "You may want to create a".yellow();
            let part4 = "file in your project root to configure bumps.\n".yellow();
            warn!(
                "{} 'cog bump' {}{} 'cog.toml' {}",
                part1, part2, part3, part4
            );
        }
        let statuses = self.repository.get_statuses()?;

        if skip_untracked || SETTINGS.skip_untracked {
            eprintln!("{}", self.repository.get_statuses()?);
        } else {
            ensure!(statuses.0.is_empty(), "{}", self.repository.get_statuses()?);
        }

        if !SETTINGS.branch_whitelist.is_empty() {
            if let Some(branch) = self.repository.get_branch_shorthand() {
                let whitelist = &SETTINGS.branch_whitelist;
                let is_match = whitelist.iter().any(|pattern| {
                    let glob = Glob::new(pattern)
                        .expect("invalid glob pattern")
                        .compile_matcher();
                    glob.is_match(&branch)
                });

                ensure!(
                    is_match,
                    "No patterns matched in {:?} for branch '{}', bump is not allowed",
                    whitelist,
                    branch
                )
            }
        };

        Ok(())
    }

    fn create_bump_commit(&self, opts: &BumpOptions, msg: impl ToString) -> Result<()> {
        if opts.disable_bump_commit || SETTINGS.disable_bump_commit {
            return Ok(());
        }

        let skip_ci = opts
            .skip_ci_override
            .as_deref()
            .or(opts.skip_ci.then_some(&SETTINGS.skip_ci));
        let message = if let Some(skip_ci) = skip_ci {
            format!("chore(version): {} {skip_ci}", msg.to_string())
        } else {
            format!("chore(version): {}", msg.to_string())
        };

        let sign = self.repository.gpg_sign();
        self.repository.commit(&message, sign, true)?;

        Ok(())
    }

    fn create_bump_tag(&self, bump: &BumpResult, annotated: Option<&str>) -> Result<()> {
        let message = annotated
            .map(|tpl| {
                let mut context = tera::Context::new();
                context.insert("latest", &bump.current.version.to_string());
                context.insert("version", &bump.next.version.to_string());
                tera::Tera::one_off(tpl, &context, false)
            })
            .transpose()?;

        self.repository.create_tag(&bump.next, message.as_deref())?;

        Ok(())
    }
}
