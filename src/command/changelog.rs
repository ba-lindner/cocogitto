use crate::conventional::changelog::release::Release;
use crate::conventional::changelog::template::MonoRepoContext;
use crate::conventional::changelog::template::PackageBumpContext;
use crate::conventional::changelog::template::Template;

use crate::conventional::changelog::ReleaseType;
use crate::git::rev::revspec::RevSpec;
use crate::CocoGitto;
use crate::SETTINGS;
use anyhow::anyhow;
use anyhow::Result;

impl CocoGitto {
    /// ## Get a changelog between two oids
    /// - `from` default value:latest tag or else first commit
    /// - `to` default value:`HEAD` or else first commit
    pub fn get_changelog(&self, pattern: &str, _with_child_releases: bool) -> Result<Release> {
        let commit_range = self.repository.revwalk_pattern(pattern)?;
        Release::try_from(commit_range).map_err(Into::into)
    }

    pub fn get_monorepo_changelog(&self, pattern: &str, template: Template) -> Result<String> {
        let mut packages = vec![];

        let package_data = SETTINGS
            .packages
            .iter()
            .map(|(name, p)| (name, p.path.to_string_lossy().to_string()));

        for (package_name, package_path) in package_data {
            let spec = RevSpec::parse(pattern, &self.repository)?;
            let cache = self.repository.get_cache();
            let to_oid_of = |oid| {
                cache
                    .get_info(oid)
                    .into_owned()
                    .as_oid_of(Some(package_name))
            };

            let context = PackageBumpContext {
                package_name,
                package_path,
                version: to_oid_of(spec.to.unwrap_or(cache.head)),
                from: Some(to_oid_of(spec.from.unwrap_or(cache.first))),
            };

            packages.push(context);
        }
        let context = MonoRepoContext {
            package_lock: false,
            packages,
        };

        let spec = RevSpec::parse(pattern, &self.repository)?;
        let mut commits = self.repository.revwalk(&spec)?;
        commits.retain_global(&self.repository)?;

        let changelog = Release::try_from(commits)?;
        changelog
            .as_markdown(template, ReleaseType::MonoRepo(context))
            .map_err(Into::into)
    }

    pub fn get_changelog_at_tag(&self, tag: &str, template: Template) -> Result<String> {
        let changelog = self.get_changelog(tag, false)?;

        changelog
            .as_markdown(template, ReleaseType::Standard)
            .map_err(|err| anyhow!(err))
    }
}
