use std::collections::HashMap;

use crate::conventional::changelog::release::Release;
use crate::conventional::changelog::template::PackageBumpContext;

use crate::git::rev::filters::CommitRangeFilter;
use crate::git::rev::CommitRange;
use crate::git::tag::Tag;
use crate::CocoGitto;
use crate::SETTINGS;
use anyhow::Result;

impl CocoGitto {
    pub fn get_changelog_at(&self, tag: &str, filter: CommitRangeFilter) -> Result<Release<'_>> {
        let range = self.repository.revwalk_pattern(tag)?;
        self.build_release(range, filter)
    }

    pub fn get_changelog_range(
        &self,
        pattern: &str,
        filter: CommitRangeFilter,
    ) -> Result<Vec<Release<'_>>> {
        self.repository
            .revwalk_pattern(pattern)?
            .split_at_tags(filter)
            .into_iter()
            .map(|range| self.build_release(range, filter))
            .collect()
    }

    fn build_release(
        &self,
        range: CommitRange<'_>,
        filter: CommitRangeFilter,
    ) -> Result<Release<'_>> {
        let package_bumps = if filter == CommitRangeFilter::Global {
            let mut package_bumps: HashMap<&String, Option<(Tag, Tag)>> = HashMap::new();
            // get `'static` lifetime from SETTINGS
            for pkg in SETTINGS.packages.keys() {
                package_bumps.insert(pkg, None);
            }
            for commit in &range {
                let tags = &self.repository.get_cache().get_info(commit.id()).tags;
                for tag in tags {
                    if let Some(pkg) = &tag.package {
                        let bump = package_bumps
                            .get_mut(pkg)
                            .expect("all packages should have been added");
                        if let Some(bump) = bump {
                            if tag < &bump.0 {
                                bump.0 = tag.clone();
                            }
                            if tag > &bump.1 {
                                bump.1 = tag.clone();
                            }
                        } else {
                            *bump = Some((tag.clone(), tag.clone()));
                        }
                    }
                }
            }
            package_bumps
                .into_iter()
                .filter_map(|(pkg, bump)| Some((pkg, bump?)))
                .map(|(pkg, (first, last))| PackageBumpContext {
                    package_name: pkg,
                    package_path: SETTINGS.packages[pkg].path.display().to_string(),
                    version: last.into(),
                    from: Some(
                        self.repository
                            .get_previous_tag(&first)
                            .map(Into::into)
                            .unwrap_or(self.repository.get_cache().first.into()),
                    ),
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut release = Release::build_single(range.filter(filter)?, filter);
        release.packages = package_bumps;
        Ok(release)
    }

    pub fn get_monorepo_changelog(&self, pattern: &str) -> Result<Vec<Release<'_>>> {
        let ranges = self
            .repository
            .revwalk_pattern(pattern)?
            .split_at_tags(CommitRangeFilter::Global);
        let mut releases = Vec::new();
        for range in ranges {
            let mut package_bumps: HashMap<&String, Option<(Tag, Tag)>> = HashMap::new();
            // get `'static` lifetime from SETTINGS
            for pkg in SETTINGS.packages.keys() {
                package_bumps.insert(pkg, None);
            }
            for commit in &range {
                let tags = &self.repository.get_cache().get_info(commit.id()).tags;
                for tag in tags {
                    if let Some(pkg) = &tag.package {
                        let bump = package_bumps
                            .get_mut(pkg)
                            .expect("all packages should have been added");
                        if let Some(bump) = bump {
                            if tag < &bump.0 {
                                bump.0 = tag.clone();
                            }
                            if tag > &bump.1 {
                                bump.1 = tag.clone();
                            }
                        } else {
                            *bump = Some((tag.clone(), tag.clone()));
                        }
                    }
                }
            }
            let mut release = Release::build_single(range, CommitRangeFilter::Global);
            release.packages = package_bumps
                .into_iter()
                .filter_map(|(pkg, bump)| Some((pkg, bump?)))
                .map(|(pkg, (first, last))| PackageBumpContext {
                    package_name: pkg,
                    package_path: SETTINGS.packages[pkg].path.display().to_string(),
                    version: last.into(),
                    from: Some(
                        self.repository
                            .get_previous_tag(&first)
                            .map(Into::into)
                            .unwrap_or(self.repository.get_cache().first.into()),
                    ),
                })
                .collect();
            releases.push(release);
        }
        Ok(releases)
    }
}
