use std::{ffi::OsStr, process::Command};

use crate::helpers::*;

use anyhow::Result;
use assert_cmd::{assert::Assert, prelude::*};
use chrono::Utc;
use cmd_lib::run_cmd;
use cocogitto::settings::Settings;
use indoc::{formatdoc, indoc};
use sealed_test::prelude::*;
use speculoos::prelude::*;
use std::path::Path;

mod hooks;
mod settings;

fn cog_bump(args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Result<Assert> {
    Ok(Command::cargo_bin("cog")?.arg("bump").args(args).assert())
}

fn cog_bump_auto() -> Result<Assert> {
    cog_bump(["--auto"])
}

#[sealed_test]
fn auto_bump_from_start_ok() -> Result<()> {
    git_init()?;
    git_commit("chore: init")?;
    git_commit("feat(taef): feature")?;
    git_commit("fix: bug fix")?;

    cog_bump_auto()?.success();

    assert_that!(Path::new("CHANGELOG.md")).exists();
    assert_tag_exists("0.1.0")?;
    Ok(())
}

#[sealed_test]
fn auto_bump_minor_from_latest_tag() -> Result<()> {
    git_init()?;
    git_commit("chore: init")?;
    git_commit("feat(taef): feature")?;
    git_commit("fix: bug fix")?;
    git_tag("1.0.0")?;
    git_commit("feat(taef): feature")?;
    git_commit("feat: feature 1")?;
    git_commit("feat: feature 2")?;

    cog_bump_auto()?.success();

    assert_that!(Path::new("CHANGELOG.md")).exists();
    assert_tag_exists("1.1.0")?;
    Ok(())
}

#[sealed_test]
fn auto_bump_major_from_latest_tag() -> Result<()> {
    git_init()?;
    git_commit("chore: init")?;
    git_commit("feat(taef): feature")?;
    git_commit("fix: bug fix")?;
    git_tag("1.0.0")?;
    git_commit("feat!(taef): feature")?;
    git_commit("feat!: feature 1")?;
    git_commit("feat: feature 2")?;

    cog_bump_auto()?.success();

    assert_that!(Path::new("CHANGELOG.md")).exists();
    assert_tag_exists("2.0.0")?;
    Ok(())
}

#[sealed_test]
fn auto_bump_patch_from_latest_tag() -> Result<()> {
    git_init()?;
    git_commit("chore: init")?;
    git_commit("feat(taef): feature")?;
    git_commit("fix: bug fix")?;
    git_tag("1.0.0")?;
    git_commit("fix(the_fix): the_fix")?;
    git_commit("fix: fix 1")?;
    git_commit("fix: fix 2")?;

    cog_bump_auto()?.success();

    assert_that!(Path::new("CHANGELOG.md")).exists();
    assert_tag_exists("1.0.1")?;
    Ok(())
}

#[sealed_test]
fn auto_bump_respect_semver_sorting() -> Result<()> {
    git_init()?;
    git_commit("chore: init")?;
    git_commit("feat(taef): feature")?;
    git_commit("fix: bug fix")?;
    git_tag("0.9.1")?;
    git_commit("feat(the_fix): feature")?;
    git_tag("0.10.0")?;
    git_commit("fix: fix 1")?;
    git_commit("fix: fix 2")?;

    cog_bump_auto()?.success();

    assert_that!(Path::new("CHANGELOG.md")).exists();
    assert_tag_exists("0.10.1")?;
    Ok(())
}

#[sealed_test]
fn minor_bump() -> Result<()> {
    git_init()?;
    git_commit("chore: init")?;
    git_tag("1.0.0")?;
    git_commit("feat: feature")?;

    cog_bump(["--minor"])?.success();

    assert_that!(Path::new("CHANGELOG.md")).exists();
    assert_tag_exists("1.1.0")?;
    Ok(())
}

#[sealed_test]
fn major_bump() -> Result<()> {
    git_init()?;
    git_commit("chore: init")?;
    git_tag("1.0.0")?;
    git_commit("feat: feature")?;

    cog_bump(["--major"])?.success();

    assert_that!(Path::new("CHANGELOG.md")).exists();
    assert_tag_exists("2.0.0")?;
    Ok(())
}

#[sealed_test]
fn patch_bump() -> Result<()> {
    git_init()?;
    git_commit("chore: init")?;
    git_tag("1.0.0")?;
    git_commit("feat: feature")?;

    cog_bump(["--patch"])?.success();

    assert_that!(Path::new("CHANGELOG.md")).exists();
    assert_tag_exists("1.0.1")?;
    Ok(())
}

#[sealed_test]
fn pre_release_bump() -> Result<()> {
    git_init()?;
    git_commit("chore: init")?;
    git_tag("1.0.0")?;
    git_commit("feat: feature")?;

    cog_bump(["--major", "--pre", "alpha"])?.success();

    assert_that!(Path::new("CHANGELOG.md")).exists();
    assert_tag_exists("2.0.0-alpha")?;
    Ok(())
}

#[sealed_test]
fn build_release_bump() -> Result<()> {
    git_init()?;
    git_commit("chore: init")?;
    git_tag("1.0.0")?;
    git_commit("feat: feature")?;

    cog_bump(["--major", "--build", "a.b.c"])?.success();

    assert_that!(Path::new("CHANGELOG.md")).exists();
    assert_tag_exists("2.0.0+a.b.c")?;
    Ok(())
}

#[sealed_test]
fn override_default_commit() -> Result<()> {
    git_init()?;

    git_add(
        indoc! {
          r#"[commit_types]
          feat = { changelog_title = "🌟 Features" }
          fix = { changelog_title = "🐛 Bug Fixes" }"#
        },
        "cog.toml",
    )?;

    git_commit("chore: init")?;
    git_commit("feat: feature")?;
    git_commit("fix: fix")?;

    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .assert()
        .success();

    assert_tag_exists("0.1.0")?;

    Ok(())
}

#[sealed_test]
fn bump_repeatedly() -> Result<()> {
    // Arrange
    git_init()?;
    git_commit("feat: first commit")?;

    // Act
    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .arg("--build")
        .arg("test.1")
        .assert()
        .success();

    run_cmd!(git reset --hard HEAD^)?;

    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .arg("--build")
        .arg("test.2")
        .assert()
        .success();

    run_cmd!(git reset --hard HEAD^)?;

    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .arg("--build")
        .arg("test.3")
        .assert()
        .success();

    // Assert
    assert_tag_exists("0.1.0+test.1")?;
    assert_tag_exists("0.1.0+test.2")?;
    assert_tag_exists("0.1.0+test.3")?;

    Ok(())
}

#[sealed_test]
fn bump_bug_fix() -> Result<()> {
    // Arrange
    git_init()?;
    git_commit("feat: first commit")?;
    git_tag("1.0.0")?;
    git_commit("feat: amazing stuff")?;
    git_commit("feat!: break everything")?;
    git_tag("2.0.0")?;
    run_cmd!(
        git branch bugfix 1.0.0;
        git switch bugfix;
    )?;
    git_commit("fix: important bug fix")?;

    // Act
    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .assert()
        .success();

    // Assert
    assert_tag_exists("1.0.1")?;

    Ok(())
}

#[sealed_test]
fn bump_from_latest_pre_release() -> Result<()> {
    // Arrange
    git_init()?;
    git_commit("chore: init")?;
    git_commit("feat: feature 1")?;
    git_tag("1.0.0-alpha.0")?;
    git_commit("feat: feature 2")?;

    // Act
    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .assert()
        .success();

    // Assert
    assert_tag_exists("1.0.0")?;

    Ok(())
}

#[sealed_test]
fn bump_prerelease_from_latest_pre_release() -> Result<()> {
    // Arrange
    git_init()?;
    git_commit("chore: init")?;
    git_commit("feat: feature 1")?;
    git_tag("1.0.0-alpha.0")?;
    git_commit("feat: feature 2")?;

    // Act
    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .arg("--pre")
        .arg("beta.0")
        .assert()
        .success();

    // Assert
    assert_tag_exists("1.0.0-beta.0")?;

    Ok(())
}

#[sealed_test]
fn bump_from_latest_pre_release_monorepo() -> Result<()> {
    // Arrange
    git_init()?;
    git_add("packages.pkg.path = \"pkg\"", "cog.toml")?;
    git_commit("chore: init")?;
    git_commit("feat: feature 1")?;
    git_tag("1.0.0-alpha.0")?;
    git_tag("pkg-1.0.0-alpha.0")?;
    std::fs::create_dir("pkg")?;
    git_add("fn main() {}", "pkg/main.rs")?;
    git_commit("feat: feature 2")?;

    // Act
    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .assert()
        .success();

    // Assert
    assert_tag_exists("1.0.0")?;
    assert_tag_exists("pkg-1.0.0")?;

    Ok(())
}

#[sealed_test]
fn bump_prerelease_from_latest_pre_release_monorepo() -> Result<()> {
    // Arrange
    git_init()?;
    git_add("packages.pkg.path = \"pkg\"", "cog.toml")?;
    git_commit("chore: init")?;
    git_commit("feat: feature 1")?;
    git_tag("1.0.0-alpha.0")?;
    git_tag("pkg-1.0.0-alpha.0")?;
    std::fs::create_dir("pkg")?;
    git_add("fn main() {}", "pkg/main.rs")?;
    git_commit("feat: feature 2")?;

    // Act
    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .arg("--pre")
        .arg("beta.0")
        .assert()
        .success();

    // Assert
    assert_tag_exists("1.0.0-beta.0")?;
    assert_tag_exists("pkg-1.0.0-beta.0")?;

    Ok(())
}

#[sealed_test]
fn bump_non_public_package() -> Result<()> {
    // Arrange
    git_init()?;
    git_add(
        indoc!(
            r#"
            [packages.pkg]
            path = "pkg"
            public_api = false
            "#
        ),
        "cog.toml",
    )?;
    git_commit("chore: init")?;
    git_add(".", "pkg/file")?;
    git_commit("feat: implement package")?;

    // Act
    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .assert()
        .success();

    // Assert
    assert_tag_exists("pkg-0.1.0")?;
    assert_tag_does_not_exist("0.0.0")?;

    Ok(())
}

#[sealed_test]
fn bump_changelog_prereleases() -> Result<()> {
    // Arrange
    git_init()?;
    git_add("commit_types.chore.omit_from_changelog = true", "cog.toml")?;
    git_commit("chore: init")?;
    let sha_1 = git_commit_short("feat: initial")?;

    // Act
    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .assert()
        .success();
    assert_tag_exists("0.1.0")?;

    let sha_2 = git_commit_short("feat: production")?;

    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--major")
        .arg("--pre")
        .arg("rc.1")
        .assert()
        .success();
    assert_tag_exists("1.0.0-rc.1")?;

    let sha_3 = git_commit_short("fix: first bug")?;

    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .arg("--pre")
        .arg("rc.2")
        .assert()
        .success();
    assert_tag_exists("1.0.0-rc.2")?;

    let sha_4 = git_commit_short("fix: second bug")?;

    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .arg("--pre")
        .arg("rc.3")
        .assert()
        .success();
    assert_tag_exists("1.0.0-rc.3")?;

    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .assert()
        .success();

    // Assert
    assert_tag_exists("1.0.0")?;
    assert_that!(std::fs::read_to_string("CHANGELOG.md"))
        .is_ok()
        .is_equal_to(formatdoc! {
            r#"
            # Changelog
            All notable changes to this project will be documented in this file. See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

            - - -
            ## 1.0.0 - {today}
            #### Bug Fixes
            - second bug - ({sha_4}) - Tom
            - first bug - ({sha_3}) - Tom
            #### Features
            - production - ({sha_2}) - Tom

            - - -

            ## 1.0.0-rc.3 - {today}
            #### Bug Fixes
            - second bug - ({sha_4}) - Tom

            - - -

            ## 1.0.0-rc.2 - {today}
            #### Bug Fixes
            - first bug - ({sha_3}) - Tom

            - - -

            ## 1.0.0-rc.1 - {today}
            #### Features
            - production - ({sha_2}) - Tom

            - - -

            ## 0.1.0 - {today}
            #### Features
            - initial - ({sha_1}) - Tom

            - - -

            Changelog generated by [cocogitto](https://github.com/cocogitto/cocogitto)."#,
            today = Utc::now().date_naive()
        });

    Ok(())
}
