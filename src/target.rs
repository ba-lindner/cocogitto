use crate::{settings::MonoRepoPackage, SETTINGS};

/// The target of a cocogitto execution
///
/// mainly used for bumping and changelogs
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// The non-monorepo target
    Standard,
    /// A single package
    Package {
        name: &'static str,
        package: &'static MonoRepoPackage,
    },
    /// The monorepo global (non-package) target
    Monorepo { manual: bool },
    /// A combination of the global target and/or package targets
    ///
    /// Similar to [`Standard`](Target::Standard), but in a monorepo
    Unified {
        manual: bool,
        global: bool,
        packages: Vec<(&'static str, &'static MonoRepoPackage)>,
    },
}

impl Target {
    pub fn from_options(package: Option<&str>, manual: bool, unified: bool) -> Self {
        if SETTINGS.packages.is_empty() {
            Self::Standard
        } else if let Some(name) = package {
            Self::package(name)
        } else if unified {
            Self::Unified {
                manual,
                global: true,
                packages: SETTINGS
                    .packages
                    .iter()
                    .map(|(name, package)| (name.as_str(), package))
                    .collect(),
            }
        } else {
            Self::Monorepo { manual }
        }
    }

    pub fn package(name: &str) -> Self {
        let (name, package) = SETTINGS
            .packages
            .iter()
            .find(|(key, _)| *key == name)
            .expect("invalid package");
        Self::Package { name, package }
    }
}
