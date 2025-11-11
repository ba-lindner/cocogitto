use crate::{
    git::{error::Git2Error, repository::Repository, tag::Tag},
    settings::MonoRepoPackage,
    SETTINGS,
};
use git2::{Commit, Diff};
use globset::{Candidate, GlobBuilder, GlobSet, GlobSetBuilder};
use once_cell::sync::Lazy;
use std::{collections::HashMap, path::Path};

#[derive(Debug)]
pub(crate) struct PackagePathFilter {
    include: GlobSet,
    exclude: GlobSet,
}

impl PackagePathFilter {
    pub(super) fn from_package(package: &MonoRepoPackage) -> Self {
        Self::new(
            package.path.to_str().expect("valid package path"),
            &package.include,
            &package.ignore,
        )
    }

    pub(super) fn is_match<P: AsRef<Path> + ?Sized>(&self, path: &P) -> bool {
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

static PATH_FILTER: Lazy<HashMap<String, PackagePathFilter>> = Lazy::new(|| {
    SETTINGS
        .packages
        .iter()
        .map(|(name, package)| (name.clone(), PackagePathFilter::from_package(package)))
        .collect()
});

impl Repository {
    pub fn is_commit_global(&self, commit: &Commit<'_>) -> Result<bool, Git2Error> {
        if commit.parent(0).is_err() {
            return Ok(true);
        }
        Ok(Self::is_diff_global(&self.get_commit_diff(commit)?))
    }

    pub fn is_commit_in_package(
        &self,
        commit: &Commit<'_>,
        package: &str,
    ) -> Result<bool, Git2Error> {
        Ok(Self::is_diff_in_package(
            &self.get_commit_diff(commit)?,
            package,
        ))
    }

    fn get_commit_diff(&'_ self, commit: &Commit<'_>) -> Result<Diff<'_>, Git2Error> {
        let parent = commit
            .parent(0)
            .ok()
            .map(|commit| commit.tree())
            .transpose()?;
        let current = commit.tree()?;
        self.0
            .diff_tree_to_tree(parent.as_ref(), Some(&current), None)
            .map_err(Into::into)
    }

    fn check_diff_paths(diff: &Diff, predicate: impl Fn(&Path) -> bool) -> bool {
        diff.deltas().any(|delta| {
            delta.old_file().path().is_some_and(&predicate)
                || delta.new_file().path().is_some_and(&predicate)
        })
    }

    fn is_diff_global(diff: &Diff) -> bool {
        let filter = &*PATH_FILTER;
        Self::check_diff_paths(diff, |path| {
            filter.values().all(|filter| !filter.is_match(path))
        })
    }

    fn is_diff_in_package(diff: &Diff, package: &str) -> bool {
        let filter = PATH_FILTER.get(package).expect("invalid package");
        Self::check_diff_paths(diff, |path| filter.is_match(path))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CommitFilter<'p> {
    /// no filtering
    Standard,
    /// global commits only
    Global,
    /// package specific commits only
    Package(&'p str),
}

impl CommitFilter<'_> {
    pub(crate) fn matches_commit(
        &self,
        repository: &Repository,
        commit: &Commit<'_>,
    ) -> Result<bool, Git2Error> {
        match self {
            CommitFilter::Standard => Ok(true),
            CommitFilter::Global => repository.is_commit_global(commit),
            CommitFilter::Package(package) => repository.is_commit_in_package(commit, package),
        }
    }

    pub(crate) fn matches_tag(&self, tag: &Tag) -> bool {
        match self {
            CommitFilter::Standard => true,
            CommitFilter::Global => tag.package.is_none(),
            CommitFilter::Package(package) => tag.package.as_deref() == Some(package),
        }
    }
}
