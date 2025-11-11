use crate::conventional::changelog::error::ChangelogError;

use serde::Serialize;

use crate::git::oid::ReleaseVersion;
use std::path::Path;
use tera::Context;

pub const MACROS_TEMPLATE: &str = include_str!("template/macro/macros.tera");
pub const MACROS_TEMPLATE_NAME: &str = "macros";

#[derive(Clone, Copy, Debug)]
pub enum TplKind {
    Default,
    Remote,
    FullHash,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TplTarget {
    Standard,
    Package,
    Monorepo,
    Unified,
}

#[derive(Clone, Copy, Debug)]
pub struct PredefinedTemplate {
    pub name: &'static str,
    pub content: &'static str,
    pub target: TplTarget,
    pub kind: TplKind,
}

macro_rules! predefined_templates {
    ($($var:ident : $name:literal $content:literal $target:ident $kind:ident),+ $(,)?) => {

        impl PredefinedTemplate {
            const ALL: &[PredefinedTemplate] = &[$(PredefinedTemplate::$var,)+];

            $(pub const $var: PredefinedTemplate = PredefinedTemplate {
                name: $name,
                content: include_str!($content),
                target: TplTarget::$target,
                kind: TplKind::$kind,
            };)+

            pub fn by_target_and_kind(target: TplTarget, kind: TplKind) -> Self {
                match (target, kind) {
                    $((TplTarget::$target, TplKind::$kind) => PredefinedTemplate::$var,)+
                }
            }
        }
    };
}

predefined_templates! {
    DEFAULT: "default" "template/simple.tera" Standard Default,
    REMOTE: "remote" "template/remote.tera" Standard Remote,
    FULL_HASH: "full_hash" "template/full_hash.tera" Standard FullHash,
    PACKAGE_DEFAULT: "package_default" "template/package_simple.tera" Package Default,
    PACKAGE_REMOTE: "package_remote" "template/package_remote.tera" Package Remote,
    PACKAGE_FULL_HASH: "package_full_hash" "template/package_full_hash.tera" Package FullHash,
    MONOERPO_DEFAULT: "monorepo_default" "template/monorepo_simple.tera" Monorepo Default,
    MONOERPO_REMOTE: "monorepo_remote" "template/monorepo_remote.tera" Monorepo Remote,
    MONOERPO_FULL_HASH: "monorepo_full_hash" "template/monorepo_full_hash.tera" Monorepo FullHash,
    UNIFIED_DEFAULT: "unified_default" "template/unified_simple.tera" Unified Default,
    UNIFIED_REMOTE: "unified_remote" "template/unified_remote.tera" Unified Remote,
    UNIFIED_FULL_HASH: "unified_full_hash" "template/unified_full_hash.tera" Unified FullHash,
}

#[derive(Debug)]
pub enum Template {
    Predefined(PredefinedTemplate),
    Custom(String),
}

impl Default for Template {
    fn default() -> Self {
        Self::Predefined(PredefinedTemplate::DEFAULT)
    }
}

impl Template {
    pub fn from_arg(value: &str) -> Result<Self, ChangelogError> {
        let predefined = PredefinedTemplate::ALL.iter().find(|tpl| tpl.name == value);
        if let Some(tpl) = predefined {
            Ok(Self::Predefined(*tpl))
        } else {
            Ok(Self::Custom(std::fs::read_to_string(value)?))
        }
    }

    pub fn default_for_target(target: TplTarget) -> Self {
        Self::Predefined(PredefinedTemplate::by_target_and_kind(
            target,
            TplKind::Default,
        ))
    }

    pub fn name(&self) -> &'static str {
        match self {
            Template::Predefined(tpl) => tpl.name,
            Template::Custom(_) => "custom",
        }
    }

    pub fn content(&self) -> &str {
        match self {
            Template::Predefined(tpl) => tpl.content,
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

#[derive(Debug)]
pub struct MonoRepoContext<'a> {
    pub package_lock: bool,
    pub packages: Vec<PackageBumpContext<'a>>,
}

#[derive(Debug, Serialize)]
pub struct PackageBumpContext<'a> {
    pub package_name: &'a str,
    pub package_path: &'a Path,
    pub version: ReleaseVersion,
    pub from: Option<ReleaseVersion>,
}

#[derive(Debug)]
pub struct PackageContext<'a> {
    pub package_name: &'a str,
}

pub trait ToContext {
    fn to_context(&self) -> Context;
}

impl ToContext for MonoRepoContext<'_> {
    fn to_context(&self) -> Context {
        let mut context = tera::Context::new();
        context.insert("package_lock", &self.package_lock);
        context.insert("packages", &self.packages);
        context
    }
}

impl ToContext for PackageContext<'_> {
    fn to_context(&self) -> Context {
        let mut context = tera::Context::new();
        context.insert("package_name", &self.package_name);
        context
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
