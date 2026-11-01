use crate::{settings::MonoRepoPackage, SETTINGS};

/// The target of a cocogitto execution
///
/// mainly used for bumping and changelogs
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub enum Target {
    /// The non-monorepo target
    #[default]
    Standard,
    /// A single package
    Package {
        name: &'static str,
        package: &'static MonoRepoPackage,
    },
    /// The monorepo global (non-package) target
    Monorepo { manual: bool },
    /// A combination of the global target and all package targets
    ///
    /// Similar to [`Standard`](Target::Standard), but in a monorepo
    Unified { manual: bool },
}

impl Target {
    pub fn from_options(package: Option<&str>, manual: bool, unified: bool) -> Self {
        if SETTINGS.list_packages().is_empty() {
            Self::Standard
        } else if let Some(name) = package {
            Self::package(name)
        } else if unified {
            Self::Unified { manual }
        } else {
            Self::Monorepo { manual }
        }
    }

    pub fn package(name: &str) -> Self {
        // use iter() + find() instead of packages[name] to get static lifetime for name
        let (name, package) = SETTINGS
            .list_packages()
            .iter()
            .find(|(key, _)| *key == name)
            .expect("invalid package");
        Self::Package { name, package }
    }

    pub fn as_package(&self) -> Option<&'static str> {
        if let Target::Package { name, .. } = self {
            Some(*name)
        } else {
            None
        }
    }
}
