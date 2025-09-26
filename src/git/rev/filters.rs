use crate::{
    git::{error::Git2Error, rev::CommitRange, tag::Tag},
    settings::MonoRepoPackage,
    SETTINGS,
};
use globset::{Candidate, GlobBuilder, GlobSet, GlobSetBuilder};
use std::path::Path;

#[derive(Debug)]
pub(crate) struct PackagePathFilter {
    include: GlobSet,
    exclude: GlobSet,
}

impl PackagePathFilter {
    pub(crate) fn from_package(package: &MonoRepoPackage) -> Self {
        Self::new(
            package.path.to_str().expect("valid package path"),
            &package.include,
            &package.ignore,
        )
    }

    pub(crate) fn is_match<P: AsRef<Path> + ?Sized>(&self, path: &P) -> bool {
        let candidate = Candidate::new(path);
        self.include.is_match_candidate(&candidate) && !self.exclude.is_match_candidate(&candidate)
    }

    fn new(package_path: &str, include_paths: &[String], exclude_paths: &[String]) -> Self {
        let include = {
            let mut builder = GlobSetBuilder::new();
            let package_path = package_path.strip_suffix("/").unwrap_or(package_path);
            builder.add(
                GlobBuilder::new(format!("{package_path}/**").as_str())
                    .literal_separator(true)
                    .build()
                    .expect("glob should be valid"),
            );
            for include in include_paths {
                builder.add(
                    GlobBuilder::new(include)
                        .literal_separator(true)
                        .build()
                        .expect("glob should be valid"),
                );
            }
            builder.build().expect("valid globset")
        };
        let exclude = {
            let mut builder = GlobSetBuilder::new();
            for exclude in exclude_paths {
                builder.add(
                    GlobBuilder::new(exclude)
                        .literal_separator(true)
                        .build()
                        .expect("glob should be valid"),
                );
            }
            builder.build().expect("valid globset")
        };

        PackagePathFilter { include, exclude }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum CommitRangeFilter<'p> {
    None,
    Global,
    Package(&'p str),
}

impl CommitRangeFilter<'_> {
    pub fn as_package(&self) -> Option<&str> {
        match self {
            CommitRangeFilter::None | CommitRangeFilter::Global => None,
            CommitRangeFilter::Package(package) => Some(package),
        }
    }

    pub fn matches_tag(&self, tag: &Tag) -> bool {
        match self {
            CommitRangeFilter::None => true,
            CommitRangeFilter::Global => tag.package.is_none(),
            CommitRangeFilter::Package(package) => tag.package.as_deref() == Some(package),
        }
    }
}

impl CommitRange<'_> {
    /// Apply `SETTINGS.ignore_merge_commits` and `SETTINGS.ignore_fixup_commits`
    pub fn apply_ignore_settings(mut self) -> Self {
        self.filter_merge_fixup_commits(
            SETTINGS.ignore_merge_commits,
            SETTINGS.ignore_fixup_commits,
        );
        self
    }

    /// Filter out merge or fixup commits
    pub fn filter_merge_fixup_commits(
        &mut self,
        ignore_merge_commits: bool,
        ignore_fixup_commits: bool,
    ) {
        if ignore_merge_commits {
            self.commits.retain(|commit| commit.parent_count() <= 1);
        }
        if ignore_fixup_commits {
            self.commits.retain(|commit| {
                commit.message().is_none()
                    || commit.message().is_some_and(|msg| {
                        !msg.starts_with("fixup!")
                            && !msg.starts_with("squash!")
                            && !msg.starts_with("amend!")
                    })
            });
        }
    }

    pub fn filter(mut self, filter: CommitRangeFilter<'_>) -> Result<Self, Git2Error> {
        match filter {
            CommitRangeFilter::None => {}
            CommitRangeFilter::Global => {
                let package_filter: Vec<_> = SETTINGS
                    .packages
                    .values()
                    .map(PackagePathFilter::from_package)
                    .collect();

                self.filter_impl(true, move |file| {
                    file.path().is_some_and(|path| {
                        package_filter.iter().all(|filter| !filter.is_match(path))
                    })
                })?;
            }
            CommitRangeFilter::Package(package) => {
                let path_filter = PackagePathFilter::from_package(
                    SETTINGS.packages.get(package).expect("package exists"),
                );

                self.filter_impl(false, move |file| {
                    file.path().is_some_and(|path| path_filter.is_match(path))
                })?;
            }
        }
        Ok(self)
    }

    pub fn global(self) -> Result<Self, Git2Error> {
        self.filter(CommitRangeFilter::Global)
    }

    pub fn package(self, package: &str) -> Result<Self, Git2Error> {
        self.filter(CommitRangeFilter::Package(package))
    }

    fn filter_impl(
        &mut self,
        retain_first: bool,
        file_filter: impl Fn(git2::DiffFile) -> bool,
    ) -> Result<(), Git2Error> {
        let should_retain = self
            .commits
            .iter()
            .map(|commit| {
                let parent = commit.parent(0).and_then(|commit| commit.tree()).ok();
                if retain_first && parent.is_none() {
                    return Ok(true);
                }

                let diff = self.repository.0.diff_tree_to_tree(
                    parent.as_ref(),
                    Some(&commit.tree()?),
                    None,
                )?;

                Ok(diff
                    .deltas()
                    .any(|delta| file_filter(delta.old_file()) || file_filter(delta.new_file())))
            })
            .collect::<Result<Vec<_>, Git2Error>>()?;
        let mut idx = 0;
        self.commits.retain(move |_| {
            let res = should_retain[idx];
            idx += 1;
            res
        });
        Ok(())
    }
}
