use crate::conventional::changelog::release::{PackageBump, Release};
use crate::git::rev::revspec::RevSpecPattern2;
use crate::{CocoGitto, Target, SETTINGS};
use anyhow::Result;

impl CocoGitto {
    /// ## Get a changelog between two oids
    /// - `from` default value:latest tag or else first commit
    /// - `to` default value:`HEAD` or else first commit
    pub fn get_changelog(
        &self,
        pattern: &str,
        target: Target,
        with_child_releases: bool,
    ) -> Result<Vec<Release<'_>>> {
        let spec = self.repository.revspec_from_str(pattern)?;
        let commit_range = self.repository.revwalk(spec)?.set_target(target)?;
        let mut releases = if with_child_releases {
            Release::build_multi(&commit_range)?
        } else {
            vec![Release::build(&commit_range)?]
        };
        if matches!(target, Target::Monorepo { .. } | Target::Unified { .. }) {
            let packages = self.find_package_data(spec)?;
            // FIXME: get package bump information on a per-release basis
            // actually difficult to do
            for release in &mut releases {
                release.packages = packages.clone();
            }
        }
        Ok(releases)
    }

    fn find_package_data(&self, spec: RevSpecPattern2) -> Result<Vec<PackageBump<'_>>> {
        let mut packages = vec![];

        let mut package_data: Vec<_> = SETTINGS
            .packages
            .iter()
            .map(|(name, p)| (name, &p.path))
            .collect();
        package_data.sort_by_key(|&(name, _)| name);

        for (package_name, package_path) in package_data.iter() {
            let range = self
                .repository
                .revwalk(spec)?
                .set_target(Target::Package { name: package_name })?;
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
        Ok(packages)
    }
}
