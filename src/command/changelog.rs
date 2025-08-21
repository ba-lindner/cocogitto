use crate::conventional::changelog::release::Release;
use crate::conventional::changelog::template::MonoRepoContext;
use crate::conventional::changelog::template::PackageBumpContext;
use crate::conventional::changelog::template::Template;

use crate::conventional::changelog::ReleaseType;
use crate::git::rev::filters::CommitRangeFilter;
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
        Release::build(commit_range, CommitRangeFilter::None).map_err(Into::into)
    }

    pub fn get_monorepo_changelog(&self, pattern: &str, template: Template) -> Result<String> {
        let mut packages = Vec::new();

        let spec = RevSpec::parse(pattern, &self.repository)?;
        let range = self.repository.revwalk(&spec)?;

        let cache = self.repository.get_cache();
        let all_tags: Vec<_> = range
            .iter()
            .flat_map(|commit| cache.get_info(commit.id()).into_owned().tags)
            .collect();

        for (package_name, package) in &SETTINGS.packages {
            let version = all_tags
                .iter()
                .filter(|tag| tag.package.as_ref() == Some(package_name))
                .max();
            let min = all_tags
                .iter()
                .filter(|tag| tag.package.as_ref() == Some(package_name))
                .min();

            let (version, from) = if let Some((vers, min)) = version.zip(min) {
                (
                    vers.clone().into(),
                    self.repository
                        .get_previous_tag(min)
                        .map(Into::into)
                        .unwrap_or(cache.first.into()),
                )
            } else {
                (
                    spec.to.unwrap_or(cache.head).into(),
                    spec.from.unwrap_or(cache.first).into(),
                )
            };

            let context = PackageBumpContext {
                package_name,
                package_path: package.path.to_string_lossy().into_owned(),
                version,
                from: Some(from),
            };

            packages.push(context);
        }
        let context = MonoRepoContext {
            package_lock: false,
            packages,
        };

        let range = range.global()?;
        Release::build(range, CommitRangeFilter::Global)?
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
