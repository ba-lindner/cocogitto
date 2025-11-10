mod error;
mod parser;

use std::collections::{HashMap, VecDeque};
use std::ops::Deref;
use std::path::Path;
use std::process::Command;
use std::str::FromStr;

use crate::hook::error::HookParseError;
use crate::hook::parser::VersionAccessToken;
use crate::{Tag, SETTINGS};
use parser::Token;

use crate::settings::{BumpProfile, HookType};
use anyhow::{anyhow, ensure, Result};
use semver::Version;

pub trait Hooks {
    fn bump_profiles(&self) -> &HashMap<String, BumpProfile>;
    fn pre_bump_hooks(&self) -> &Vec<String>;
    fn post_bump_hooks(&self) -> &Vec<String>;

    fn get_hooks(&self, hook_type: HookType) -> &Vec<String> {
        match hook_type {
            HookType::PreBump => self.pre_bump_hooks(),
            HookType::PostBump => self.post_bump_hooks(),
        }
    }

    fn get_profile_hooks(&self, profile: &str, hook_type: HookType) -> &Vec<String> {
        let profile = self
            .bump_profiles()
            .get(profile)
            .expect("Bump profile not found");
        match hook_type {
            HookType::PreBump => &profile.pre_bump_hooks,
            HookType::PostBump => &profile.post_bump_hooks,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct VersionSpan {
    tokens: VecDeque<Token>,
    default_version: Option<Version>,
}

#[derive(Debug)]
pub(crate) struct HookVersion {
    pub prefixed_tag: Tag,
}

impl HookVersion {
    pub(crate) fn new(tag: Tag) -> Self {
        HookVersion { prefixed_tag: tag }
    }
}

impl VersionSpan {
    pub(crate) fn build_version_str(
        &self,
        current: Option<&Tag>,
        next: Option<&Tag>,
    ) -> Result<String> {
        let mut tokens = self.tokens.iter();
        // According to the pest grammar, a `version` or `latest_version` token is expected first
        let (tag, strip_meta, err) = match tokens.next() {
            Some(Token::Version) => (next, true, "version"),
            Some(Token::LatestVersion) => (current, true, "latest"),
            Some(Token::VersionTag) => (next, false, "version_tag"),
            Some(Token::LatestVersionTag) => (current, false, "latest_tag"),
            Some(Token::Package) => {
                return next
                    .and_then(|tag| tag.package.clone())
                    .ok_or_else(|| anyhow!("Current tag as no {{{{package}}}} info"))
            }

            _ => unreachable!("Unexpected parsing error"),
        };
        let tag = tag.filter(|tag| !tag.is_zero());

        let mut tag = if let Some(tag) = tag {
            tag.clone()
        } else {
            self.default_version
                .clone()
                .map(|version| Tag {
                    package: None,
                    prefix: SETTINGS.tag_prefix.clone(),
                    version,
                    oid: None,
                })
                .ok_or_else(|| anyhow!("No previous tag found to replace {{{{{err}}}}} version"))?
        };
        if strip_meta {
            tag.prefix = None;
            tag.package = None;
        }

        let mut amount = 1;
        let mut version_access_token: Option<VersionAccessToken> = None;

        for token in tokens {
            match token {
                // reset the increment amount to default whenever we encounter a `+` token
                Token::Add => amount = 1,
                // set the desired amount
                Token::Amount(amt) => amount = *amt,
                // increments ...
                Token::Major => {
                    tag.version.major += amount;
                    tag.version.minor = 0;
                    tag.version.patch = 0;
                }
                Token::Minor => {
                    tag.version.minor += amount;
                    tag.version.patch = 0;
                }
                Token::Patch => tag.version.patch += amount,
                // set  build metadata and prerelease
                Token::PreRelease(pre_release) => tag.version.pre = pre_release.clone(),
                Token::BuildMetadata(build) => tag.version.build = build.clone(),
                Token::VersionAccess(version_access) => {
                    version_access_token = Some(*version_access);
                }
                _ => unreachable!("Unexpected parsing error"),
            }
        }

        if let Some(version_access) = version_access_token {
            Ok(match version_access {
                VersionAccessToken::Major => tag.version.major.to_string(),
                VersionAccessToken::Minor => tag.version.minor.to_string(),
                VersionAccessToken::Patch => tag.version.patch.to_string(),
            })
        } else {
            Ok(tag.to_string())
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum HookPart {
    Version(VersionSpan),
    Other(String),
}

#[derive(Debug)]
pub struct Hook(Vec<HookPart>);

impl FromStr for Hook {
    type Err = HookParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parser::parse(s).map(Self)
    }
}

impl Hook {
    pub(crate) fn insert_versions(
        &self,
        current: Option<&Tag>,
        next: Option<&Tag>,
    ) -> Result<HookCmd> {
        let mut res = String::new();
        for part in &self.0 {
            match part {
                HookPart::Version(span) => res += &span.build_version_str(current, next)?,
                HookPart::Other(s) => res += s,
            }
        }
        Ok(HookCmd(res))
    }
}

pub struct HookCmd(String);

impl Deref for HookCmd {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl HookCmd {
    pub fn run(&self, package_path: Option<&Path>) -> Result<()> {
        let mut cmd = Command::new("sh");
        let cmd = cmd.arg("-c").arg(&self.0);
        if let Some(current_dir) = package_path {
            cmd.current_dir(current_dir);
        }
        let status = cmd.status()?;
        ensure!(status.success(), "hook failed with status {}", status);
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use cmd_lib::run_cmd;
    use std::collections::{HashMap, VecDeque};
    use std::process::Command;
    use std::str::FromStr;

    use crate::hook::parser::Token;
    use crate::{Result, Tag};

    use crate::hook::{Hook, HookPart, VersionSpan};
    use crate::settings::{MonoRepoPackage, Settings};
    use crate::test_helpers::git_init_no_gpg;
    use sealed_test::prelude::*;
    use semver::Version;
    use speculoos::prelude::*;

    #[test]
    fn parse_empty_string() {
        let empty_hook = Hook::from_str("");
        assert_that!(empty_hook).is_err();
    }

    #[test]
    fn parse_valid_string() -> Result<()> {
        let hook = Hook::from_str("cargo bump {{version}}")?;
        assert_that!(hook.0).is_equal_to(vec![
            HookPart::Other("cargo bump ".to_string()),
            HookPart::Version(VersionSpan {
                tokens: VecDeque::from(vec![Token::Version]),
                default_version: None,
            }),
        ]);
        Ok(())
    }

    #[test]
    fn parse_current_version() -> Result<()> {
        let hook = Hook::from_str("cargo bump {{version_tag}}");
        assert_that!(hook).is_ok();
        Ok(())
    }

    #[test]
    fn parse_latest_tag() -> Result<()> {
        let hook = Hook::from_str("cargo bump {{latest_tag}}");
        assert_that!(hook).is_ok();
        Ok(())
    }

    #[test]
    fn replace_version_cargo() -> Result<()> {
        let hook = Hook::from_str("cargo bump {{version}}")?;
        let cmd = hook.insert_versions(None, Some(&Tag::from_str("1.0.0", None)?))?;

        assert_that!(cmd.as_str()).is_equal_to("cargo bump 1.0.0");
        Ok(())
    }

    #[test]
    fn replace_version_tag_cargo() -> Result<()> {
        let hook = Hook::from_str("cargo bump {{version_tag}}")?;
        let tag = Tag {
            package: None,
            prefix: Some("v".to_string()),
            version: Version::new(1, 0, 0),
            oid: None,
        };

        let cmd = hook.insert_versions(None, Some(&tag)).unwrap();

        assert_that!(cmd.as_str()).is_equal_to("cargo bump v1.0.0");
        Ok(())
    }

    #[sealed_test]
    fn replace_version_tag_with_package() -> Result<()> {
        let mut packages = HashMap::new();
        packages.insert("cog".to_string(), MonoRepoPackage::default());
        let settings = Settings {
            packages,
            ..Default::default()
        };

        let settings = toml::to_string(&settings)?;

        let _ = git_init_no_gpg()?;

        run_cmd!(
            echo $settings > cog.toml;
            git add .;
            git commit -m "first commit";
        )?;

        let hook = Hook::from_str("echo {{version_tag}}")?;

        let tag = Tag {
            package: Some("cog".to_string()),
            prefix: Some("v".to_string()),
            version: Version::new(1, 0, 0),
            oid: None,
        };

        let cmd = hook.insert_versions(None, Some(&tag)).unwrap();

        assert_that!(cmd.as_str()).is_equal_to("echo cog-v1.0.0");
        Ok(())
    }

    #[test]
    fn replace_latest_tag() -> Result<()> {
        let hook = Hook::from_str("echo {{latest_tag}}")?;
        let tag = Tag {
            package: None,
            prefix: Some("v".to_string()),
            version: Version::new(1, 0, 0),
            oid: None,
        };

        let cmd = hook.insert_versions(Some(&tag), None).unwrap();

        assert_that!(cmd.as_str()).is_equal_to("echo v1.0.0");
        Ok(())
    }

    #[test]
    fn replace_maven_version() -> Result<()> {
        let hook = Hook::from_str("mvn versions:set -DnewVersion={{version}}")?;
        let cmd = hook
            .insert_versions(None, Some(&Tag::from_str("1.0.0", None)?))
            .unwrap();

        assert_that!(cmd.as_str()).is_equal_to("mvn versions:set -DnewVersion=1.0.0");
        Ok(())
    }

    #[test]
    fn replace_maven_version_with_expression() -> Result<()> {
        let hook = Hook::from_str("mvn versions:set -DnewVersion={{version+1minor-SNAPSHOT}}")?;
        let cmd = hook
            .insert_versions(None, Some(&Tag::from_str("1.0.0", None)?))
            .unwrap();

        assert_that!(cmd.as_str()).is_equal_to("mvn versions:set -DnewVersion=1.1.0-SNAPSHOT");
        Ok(())
    }

    #[test]
    fn replace_version_tag_with_expression() -> Result<()> {
        let hook = Hook::from_str("mvn versions:set -DnewVersion={{version_tag+1minor-SNAPSHOT}}")?;
        let tag = Tag {
            package: None,
            prefix: Some("v".to_string()),
            version: Version::new(1, 0, 0),
            oid: None,
        };

        let cmd = hook.insert_versions(None, Some(&tag)).unwrap();

        assert_that!(cmd.as_str()).is_equal_to("mvn versions:set -DnewVersion=v1.1.0-SNAPSHOT");
        Ok(())
    }

    #[sealed_test]
    fn replace_package_version_tag_with_expression() -> Result<()> {
        let mut packages = HashMap::new();
        packages.insert("cog".to_string(), MonoRepoPackage::default());
        let settings = Settings {
            packages,
            ..Default::default()
        };

        let settings = toml::to_string(&settings)?;

        let _ = git_init_no_gpg()?;

        run_cmd!(
            echo $settings > cog.toml;
            git add .;
            git commit -m "first commit";
        )?;

        let hook = Hook::from_str("mvn versions:set -DnewVersion={{version_tag+1minor-SNAPSHOT}}")?;

        let tag = Tag {
            package: Some("cog".to_string()),
            prefix: Some("v".to_string()),
            version: Version::new(1, 0, 0),
            oid: None,
        };

        let cmd = hook.insert_versions(None, Some(&tag)).unwrap();

        assert_that!(cmd.as_str()).is_equal_to("mvn versions:set -DnewVersion=cog-v1.1.0-SNAPSHOT");
        Ok(())
    }

    #[test]
    fn leave_hook_untouched_when_no_version() -> Result<()> {
        let hook = Hook::from_str("echo \"Hello World\"")?;
        let cmd = hook
            .insert_versions(None, Some(&Tag::from_str("1.0.0", None)?))
            .unwrap();

        assert_that!(cmd.as_str()).is_equal_to("echo \"Hello World\"");
        Ok(())
    }

    #[test]
    fn replace_quoted_version() -> Result<()> {
        let hook = Hook::from_str("echo \"{{version}}\"")?;
        let cmd = hook
            .insert_versions(None, Some(&Tag::from_str("1.0.0", None)?))
            .unwrap();

        assert_that!(cmd.as_str()).is_equal_to("echo \"1.0.0\"");
        Ok(())
    }

    #[test]
    fn replace_version_with_nested_simple_quoted_arg() -> Result<()> {
        let hook = Hook::from_str("cog commit chore 'bump snapshot to {{version+1minor-pre}}'")?;
        let cmd = hook
            .insert_versions(None, Some(&Tag::from_str("1.0.0", None)?))
            .unwrap();

        assert_that!(cmd.as_str()).is_equal_to("cog commit chore 'bump snapshot to 1.1.0-pre'");
        Ok(())
    }

    #[test]
    fn replace_version_with_nested_double_quoted_arg() -> Result<()> {
        let hook = Hook::from_str("cog commit chore \"bump snapshot to {{version+1minor-pre}}\"")?;
        let cmd = hook
            .insert_versions(None, Some(&Tag::from_str("1.0.0", None)?))
            .unwrap();

        assert_that!(cmd.as_str()).is_equal_to("cog commit chore \"bump snapshot to 1.1.0-pre\"");
        Ok(())
    }

    #[test]
    fn replace_version_with_multiple_placeholders() -> Result<()> {
        let hook = Hook::from_str("echo \"the latest {{latest}}, the greatest {{version}}\"")?;
        let cmd = hook
            .insert_versions(
                Some(&Tag::from_str("0.5.9", None)?),
                Some(&Tag::from_str("1.0.0", None)?),
            )
            .unwrap();

        assert_that!(cmd.as_str()).is_equal_to("echo \"the latest 0.5.9, the greatest 1.0.0\"");
        Ok(())
    }

    #[test]
    fn replace_version_with_multiple_placeholders_and_increments() -> Result<()> {
        let hook = Hook::from_str(
            "echo \"the latest {{latest+3major+1minor}}, the greatest {{version+2patch}}\"",
        )?;
        let cmd = hook
            .insert_versions(
                Some(&Tag::from_str("0.5.9", None)?),
                Some(&Tag::from_str("1.0.0", None)?),
            )
            .unwrap();

        assert_that!(cmd.as_str()).is_equal_to("echo \"the latest 3.1.0, the greatest 1.0.2\"");
        Ok(())
    }

    #[test]
    fn replace_version_with_pre_and_build_metadata() -> Result<()> {
        let hook =
            Hook::from_str("echo \"the latest {{version+1major-pre.alpha-bravo+build.42}}\"")?;
        let cmd = hook
            .insert_versions(None, Some(&Tag::from_str("1.0.0", None)?))
            .unwrap();

        assert_that!(cmd.as_str())
            .is_equal_to("echo \"the latest 2.0.0-pre.alpha-bravo+build.42\"");
        Ok(())
    }

    #[test]
    fn replaces_version_with_default_when_no_version_available() -> Result<()> {
        let hook = Hook::from_str("echo \"the latest {{version|1.0.0}}\"")?;
        let cmd = hook.insert_versions(None, None).unwrap();

        assert_that!(cmd.as_str()).is_equal_to("echo \"the latest 1.0.0\"");
        Ok(())
    }

    #[test]
    fn replaces_version_with_default_with_modifiers_when_no_version_available() -> Result<()> {
        let hook = Hook::from_str(
            "echo \"the latest {{version|1.0.0+1major-pre.alpha-bravo+build.42}}\"",
        )?;
        let cmd = hook.insert_versions(None, None).unwrap();

        assert_that!(cmd.as_str())
            .is_equal_to("echo \"the latest 2.0.0-pre.alpha-bravo+build.42\"");
        Ok(())
    }

    #[sealed_test]
    fn replaces_version_tag_with_default_when_no_version_tag_available() -> Result<()> {
        let settings = Settings {
            tag_prefix: Some("v".to_string()),
            ..Default::default()
        };
        let settings = toml::to_string(&settings)?;

        let _ = git_init_no_gpg()?;

        run_cmd!(
            echo $settings > cog.toml;
        )?;

        let hook = Hook::from_str("echo \"the latest {{version_tag|1.0.0}}\"")?;
        let cmd = hook.insert_versions(None, None).unwrap();

        assert_that!(cmd.as_str()).is_equal_to("echo \"the latest v1.0.0\"");
        Ok(())
    }

    #[test]
    fn replace_version_tag_with_pre_and_build_metadata() -> Result<()> {
        let hook =
            Hook::from_str("echo \"the latest {{version_tag+1major-pre.alpha-bravo+build.42}}\"")?;

        let tag = Tag {
            package: None,
            prefix: Some("v".to_string()),
            version: Version::new(1, 0, 0),
            oid: None,
        };

        let cmd = hook.insert_versions(None, Some(&tag)).unwrap();

        assert_that!(cmd.as_str())
            .is_equal_to("echo \"the latest v2.0.0-pre.alpha-bravo+build.42\"");

        Ok(())
    }

    #[sealed_test]
    fn parenthesis_in_hook_works() -> Result<()> {
        git_init_no_gpg()?;

        let hook = Hook::from_str("git commit --allow-empty -m 'chore(snapshot): bump snapshot to {{version+1patch-SNAPSHOT}}'")?;

        let cmd = hook
            .insert_versions(None, Some(&Tag::from_str("1.0.0", None)?))
            .unwrap();

        let outcome = Command::new("sh").arg("-c").arg(&*cmd).status();

        assert_that!(outcome)
            .is_ok()
            .matches(|status| status.success());

        Ok(())
    }

    #[sealed_test]
    fn replace_package_name_and_version_tag_with_expression() -> Result<()> {
        let mut packages = HashMap::new();
        packages.insert("cog".to_string(), MonoRepoPackage::default());
        let settings = Settings {
            packages,
            ..Default::default()
        };

        let settings = toml::to_string(&settings)?;

        let _ = git_init_no_gpg();
        run_cmd!(
            echo $settings > cog.toml;
            git add .;
            git commit -m "first commit";
        )?;

        let hook = Hook::from_str(
            r#"echo "{{package}}, version: {{version}}, tag: {{version_tag}}, current: {{latest}}, current_tag: {{latest_tag}}""#,
        )?;

        let current = Tag {
            package: Some("cog".to_string()),
            prefix: Some("v".to_string()),
            version: Version::new(1, 0, 0),
            oid: None,
        };

        let tag = Tag {
            package: Some("cog".to_string()),
            prefix: Some("v".to_string()),
            version: Version::new(1, 1, 0),
            oid: None,
        };

        let cmd = hook.insert_versions(Some(&current), Some(&tag)).unwrap();

        assert_that!(cmd.as_str())
            .is_equal_to(r#"echo "cog, version: 1.1.0, tag: cog-v1.1.0, current: 1.0.0, current_tag: cog-v1.0.0""#);
        Ok(())
    }

    #[sealed_test]
    fn replace_major_minor_patch() -> Result<()> {
        let mut packages = HashMap::new();
        packages.insert("cog".to_string(), MonoRepoPackage::default());
        let settings = Settings {
            packages,
            ..Default::default()
        };

        let settings = toml::to_string(&settings)?;

        let _ = git_init_no_gpg()?;

        run_cmd!(
            echo $settings > cog.toml;
            git add .;
            git commit -m "first commit";
        )?;

        let hook = Hook::from_str(
            r#"major={{version.major}} minor={{version.minor}} patch={{version.patch}}"#,
        )?;

        let current = Tag {
            package: Some("cog".to_string()),
            prefix: Some("v".to_string()),
            version: Version::new(1, 0, 0),
            oid: None,
        };

        let tag = Tag {
            package: Some("cog".to_string()),
            prefix: Some("v".to_string()),
            version: Version::new(1, 2, 3),
            oid: None,
        };

        let cmd = hook.insert_versions(Some(&current), Some(&tag)).unwrap();

        assert_that!(cmd.as_str()).is_equal_to(r#"major=1 minor=2 patch=3"#);
        Ok(())
    }
}
