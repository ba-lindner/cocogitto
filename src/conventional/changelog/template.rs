use crate::conventional::changelog::error::ChangelogError;
use crate::conventional::changelog::release::ReleaseVersion;

use serde::Serialize;

use std::path::PathBuf;
use tera::Context;

#[derive(Debug, Clone)]
pub enum Template {
    Predefined(&'static str, &'static str),
    Custom(String),
}

macro_rules! predefined {
    ($($var:ident $name:literal $path:literal,)+) => {
        impl Template {
            $(pub const $var: Template = Template::Predefined($name, include_str!($path));)+

            const PREDEFINED: [Template; 9] = [$(Self::$var,)+];
        }
    };
}

predefined! {
    DEFAULT "default" "template/simple",
    REMOTE "remote" "template/remote",
    FULL_HASH "full_hash" "template/full_hash",
    PACKAGE_DEFAULT "package_default" "template/package_simple",
    PACKAGE_REMOTE "package_remote" "template/package_remote",
    PACKAGE_FULL_HASH "package_full_hash" "template/package_full_hash",
    MONOREPO_DEFAULT "monorepo_default" "template/monorepo_simple",
    MONOREPO_REMOTE "monorepo_remote" "template/monorepo_remote",
    MONOREPO_FULL_HASH "monorepo_full_hash" "template/monorepo_full_hash",
}

impl Default for Template {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Template {
    pub fn new(s: &str) -> Result<Self, ChangelogError> {
        for template in Self::PREDEFINED {
            if template.name() == s {
                return Ok(template);
            }
        }
        let path = PathBuf::from(s);
        if let Ok(content) = std::fs::read_to_string(&path) {
            Ok(Template::Custom(content))
        } else {
            Err(ChangelogError::TemplateNotFound(path))
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Template::Predefined(name, _) => name,
            Template::Custom(_) => "custom_template",
        }
    }

    pub fn content(&self) -> &str {
        match self {
            Template::Predefined(_, content) => content,
            Template::Custom(content) => content,
        }
    }
}

/// A wrapper to append remote repository information to template context
#[derive(Debug)]
pub struct RemoteContext {
    remote: String,
    repository: String,
    owner: String,
}

#[derive(Debug, Serialize)]
pub struct MonoRepoContext {
    pub package_lock: bool,
}

#[derive(Debug, Serialize)]
pub struct PackageBumpContext<'a> {
    pub package_name: &'a str,
    pub package_path: String,
    pub version: ReleaseVersion,
    pub from: Option<ReleaseVersion>,
}

#[derive(Debug, Serialize)]
pub struct PackageContext<'a> {
    pub package_name: &'a str,
}

pub trait ToContext {
    fn to_context(&self) -> Context;
}

impl<T: Serialize> ToContext for T {
    fn to_context(&self) -> Context {
        Context::from_serialize(self).expect("failed to serialize")
    }
}

pub struct Empty;
impl ToContext for Empty {
    fn to_context(&self) -> Context {
        Context::new()
    }
}

impl ToContext for RemoteContext {
    fn to_context(&self) -> Context {
        let mut context = tera::Context::new();
        context.insert("platform", &format!("https://{}", self.remote.as_str()));
        context.insert("owner", self.owner.as_str());
        context.insert(
            "repository_url",
            &format!("https://{}/{}/{}", self.remote, self.owner, self.repository),
        );

        context
    }
}

impl RemoteContext {
    pub fn try_new(
        remote: Option<String>,
        repository: Option<String>,
        owner: Option<String>,
    ) -> Option<Self> {
        match (remote, repository, owner) {
            (Some(remote), Some(repository), Some(owner)) => Some(Self {
                remote,
                repository,
                owner,
            }),
            (None, None, None) => None,
            _ => panic!("Changelog remote context should be set. Missing one of 'remote', 'repository', 'owner' in changelog configuration")
        }
    }
}
