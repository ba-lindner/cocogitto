use crate::conventional::changelog::release::PackageBump;
use crate::conventional::changelog::release::Release;
use crate::conventional::changelog::template::MonoRepoContext;
use crate::conventional::changelog::Changelog;

use crate::conventional::changelog::ReleaseType;
use crate::git::rev::filters::CommitFilter;
use crate::git::rev::CommitIter;
use crate::CocoGitto;
use crate::SETTINGS;
use anyhow::Result;

impl CocoGitto {
    /// ## Get a changelog between two oids
    /// - `from` default value:latest tag or else first commit
    /// - `to` default value:`HEAD` or else first commit
    pub fn get_changelog(&self, pattern: &str, with_child_releases: bool) -> Result<Changelog<'_>> {
        let commit_range = self
            .repository
            .revwalk(self.repository.revspec_from_str(pattern)?)?;
        Self::build_changelog(&commit_range, with_child_releases, ReleaseType::Standard)
    }

    fn build_changelog<'r>(
        commits: &'_ CommitIter,
        with_child_releases: bool,
        context: ReleaseType<'r>,
    ) -> Result<Changelog<'r>> {
        let releases = if with_child_releases {
            Release::build_multi(commits)?
        } else {
            vec![Release::build(commits)?]
        };
        Ok(Changelog { releases, context })
    }

    pub fn get_monorepo_changelog(
        &self,
        pattern: &str,
        unified: bool,
        with_child_releases: bool,
    ) -> Result<Changelog<'_>> {
        let mut packages = vec![];

        let mut package_data: Vec<_> = SETTINGS
            .packages
            .iter()
            .map(|(name, p)| (name, &p.path))
            .collect();
        package_data.sort_by_key(|&(name, _)| name);

        let pattern = self.repository.revspec_from_str(pattern)?;
        for (package_name, package_path) in package_data.iter() {
            let range = self
                .repository
                .revwalk(pattern)?
                .apply_filter(CommitFilter::Package(package_name))?;
            if range.is_empty() {
                continue;
            }

            let (from, version) = range.version_range();

            let context = PackageBump {
                package_name,
                package_path,
                version,
                from: Some(from),
            };

            packages.push(context);
        }

        let commit_range = self.repository.revwalk(pattern)?.apply_filter(if unified {
            CommitFilter::Standard
        } else {
            CommitFilter::Global
        })?;

        let mut changelog = Self::build_changelog(
            &commit_range,
            with_child_releases,
            ReleaseType::MonoRepo(MonoRepoContext {
                package_lock: false,
            }),
        )?;
        // FIXME: get package bump information on a per-release basis
        // actually difficult to do
        for release in &mut changelog.releases {
            release.packages = packages.clone();
        }
        Ok(changelog)
    }

    pub fn get_changelog_at_tag(&self, tag: &str) -> Result<Changelog<'_>> {
        self.get_changelog(tag, false)
    }
}
