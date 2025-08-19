use crate::{
    git::{error::Git2Error, repository::Repository, rev::CommitIter},
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
pub(crate) enum CommitRangeFilter<'p> {
    None,
    Global,
    Package(&'p str),
}

impl CommitIter<'_> {
    /// Apply `SETTINGS.ignore_merge_commits` and `SETTINGS.ignore_fixup_commits`
    pub fn apply_ignore_settings(&mut self) {
        self.filter_merge_fixup_commits(
            SETTINGS.ignore_merge_commits,
            SETTINGS.ignore_fixup_commits,
        );
    }

    /// Filter out merge or fixup commits
    pub fn filter_merge_fixup_commits(
        &mut self,
        ignore_merge_commits: bool,
        ignore_fixup_commits: bool,
    ) {
        if ignore_merge_commits {
            self.0.retain(|(commit, _)| commit.parent_count() <= 1);
        }
        if ignore_fixup_commits {
            self.0.retain(|(commit, _)| {
                commit.message().is_none()
                    || commit.message().is_some_and(|msg| {
                        !msg.starts_with("fixup!")
                            && !msg.starts_with("squash!")
                            && !msg.starts_with("amend!")
                    })
            });
        }
    }

    pub fn filter(
        &mut self,
        repository: &Repository,
        filter: CommitRangeFilter<'_>,
    ) -> Result<(), Git2Error> {
        match filter {
            CommitRangeFilter::None => {}
            CommitRangeFilter::Global => self.retain_global(repository)?,
            CommitRangeFilter::Package(package) => self.retain_package(repository, package)?,
        }
        Ok(())
    }

    pub fn retain_global(&mut self, repository: &Repository) -> Result<(), Git2Error> {
        for (_, info) in &mut self.0 {
            info.tags.retain(|tag| tag.package.is_none());
        }

        let package_filter: Vec<_> = SETTINGS
            .packages
            .values()
            .map(PackagePathFilter::from_package)
            .collect();
        let file_filter = move |file: git2::DiffFile| {
            file.path()
                .is_some_and(|path| package_filter.iter().all(|filter| !filter.is_match(path)))
        };

        self.try_retain(move |commit| {
            let Ok(parent) = commit.parent(0) else {
                return Ok(true);
            };

            let diff = repository.0.diff_tree_to_tree(
                Some(&parent.tree()?),
                Some(&commit.tree()?),
                None,
            )?;

            Ok(diff
                .deltas()
                .any(|delta| file_filter(delta.old_file()) || file_filter(delta.new_file())))
        })
    }

    pub fn retain_package(
        &mut self,
        repository: &Repository,
        package: &str,
    ) -> Result<(), Git2Error> {
        for (_, info) in &mut self.0 {
            info.tags
                .retain(|tag| tag.package.as_deref() == Some(package));
        }
        let package = SETTINGS.packages.get(package).expect("package exists");
        let path_filter = PackagePathFilter::from_package(package);
        let file_filter =
            move |file: git2::DiffFile| file.path().is_some_and(|path| path_filter.is_match(path));

        self.try_retain(move |commit| {
            let parent = commit.parent(0).and_then(|commit| commit.tree()).ok();

            let diff =
                repository
                    .0
                    .diff_tree_to_tree(parent.as_ref(), Some(&commit.tree()?), None)?;

            Ok(diff
                .deltas()
                .any(|delta| file_filter(delta.old_file()) || file_filter(delta.new_file())))
        })
    }

    fn try_retain(
        &mut self,
        mut filter: impl FnMut(&git2::Commit) -> Result<bool, Git2Error>,
    ) -> Result<(), Git2Error> {
        let should_retain = self
            .0
            .iter()
            .map(|(commit, _)| filter(commit))
            .collect::<Result<Vec<_>, _>>()?;
        let mut idx = 0;
        self.0.retain(move |_| {
            let res = should_retain[idx];
            idx += 1;
            res
        });
        Ok(())
    }
}
