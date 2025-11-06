use serde::de::StdError;
use std::fmt::{Debug, Display, Formatter};
use std::io;
use std::path::PathBuf;

use crate::git::error::Git2Error;

#[derive(Debug)]
pub enum ChangelogError {
    TemplateNotFound(PathBuf),
    TeraError(tera::Error),
    WriteError(io::Error),
    SeparatorNotFound(PathBuf),
    EmptyRelease,
    GitError(Git2Error),
}

impl Display for ChangelogError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ChangelogError::TemplateNotFound(path) => {
                writeln!(f, "changelog template not found in {path:?}")
            }
            ChangelogError::TeraError(err) => {
                writeln!(f, "failed to render changelog: \n\t{err:?}")
            }
            ChangelogError::WriteError(err) => {
                writeln!(f, "failed to write changelog: \n\t{err}")
            }
            ChangelogError::SeparatorNotFound(path) => writeln!(
                f,
                "cannot find default separator '- - -' in {}",
                path.as_path().display()
            ),
            ChangelogError::EmptyRelease => writeln!(f, "No commit found to create a changelog",),
            ChangelogError::GitError(err) => writeln!(f, "failed to build changelog: \n\t{err}"),
        }
    }
}

impl From<io::Error> for ChangelogError {
    fn from(err: io::Error) -> Self {
        Self::WriteError(err)
    }
}

impl From<tera::Error> for ChangelogError {
    fn from(err: tera::Error) -> Self {
        Self::TeraError(err)
    }
}

impl From<Git2Error> for ChangelogError {
    fn from(err: Git2Error) -> Self {
        Self::GitError(err)
    }
}

impl StdError for ChangelogError {}
