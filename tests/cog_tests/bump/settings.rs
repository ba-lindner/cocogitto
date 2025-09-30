//! Test related to settings (cog.toml and cli options) affecting `cog bump`
//!
//! Options not tested here:
//! * version selection: core options, tested in [super]
//! * target selection: tested in [super::targets]

use super::*;

// ##################
//     Dry Run
// ##################

#[sealed_test]
fn auto_bump_dry_run_from_latest_tag() -> Result<()> {
    git_init()?;
    git_commit("chore: init")?;
    git_commit("feat(taef): feature")?;
    git_commit("fix: bug fix")?;
    git_tag("1.0.0")?;
    git_commit("feat(taef): feature")?;
    git_commit("feat: feature 1")?;
    git_commit("feat: feature 2")?;

    cog_bump(["--auto", "--dry-run"])?
        .success()
        .stdout("1.1.0\n\n");

    assert_that!(Path::new("CHANGELOG.md")).does_not_exist();
    assert_tag_does_not_exist("1.1.0")?;
    Ok(())
}

#[sealed_test]
fn monorepo_dry_run() -> Result<()> {
    init_monorepo(&mut Settings::default())?;

    cog_bump(["--auto", "--dry-run"])?
        .success()
        .stdout("one-0.1.0\n0.1.0\n\n");

    assert_that!(Path::new("CHANGELOG.md")).does_not_exist();
    assert_tag_does_not_exist("1.1.0")?;
    Ok(())
}

#[sealed_test]
fn package_dry_run() -> Result<()> {
    init_monorepo(&mut Settings::default())?;

    cog_bump(["--auto", "--dry-run", "--package", "one"])?
        .success()
        .stdout("one-0.1.0\n\n");

    assert_that!(Path::new("CHANGELOG.md")).does_not_exist();
    assert_tag_does_not_exist("1.1.0")?;
    Ok(())
}

// ##################
//     Tag Prefix
// ##################

#[sealed_test]
fn auto_bump_with_prefix() -> Result<()> {
    git_init()?;
    git_add("tag_prefix = \"v\"", "cog.toml")?;
    git_commit("chore: init")?;
    git_commit("feat(taef): feature")?;
    git_commit("fix: bug fix")?;
    git_tag("v1.0.0")?;
    git_commit("feat(taef)!: feature")?;
    git_commit("feat!: feature 1")?;
    git_commit("feat: feature 2")?;

    cog_bump_auto()?.success();

    assert_that!(Path::new("CHANGELOG.md")).exists();
    assert_tag_exists("v2.0.0")?;
    Ok(())
}

// ##################
//     Disable Changelog
// ##################

#[sealed_test]
fn disable_changelog_disables_changelog_generation() -> Result<()> {
    git_init()?;
    git_add("disable_changelog = true", "cog.toml")?;
    git_commit("chore: first commit")?;
    git_commit("feat: add a feature commit")?;
    git_tag("1.0.0")?;
    git_commit("feat: add another feature commit")?;

    cog_bump_auto()?.success();

    assert_that!(Path::new("CHANGELOG.md")).does_not_exist();
    assert_tag_exists("1.0.0")?;
    assert_tag_exists("1.1.0")?;
    Ok(())
}

#[sealed_test]
fn disable_changelog_disables_changelog_generation_for_monorepos() -> Result<()> {
    let mut settings = Settings {
        disable_changelog: true,
        ..Default::default()
    };
    init_monorepo(&mut settings)?;

    cog_bump_auto()?.success();

    assert_that!(Path::new("CHANGELOG.md")).does_not_exist();
    assert_tag_exists("0.1.0")?;
    assert_tag_exists("one-0.1.0")?;
    Ok(())
}

#[sealed_test]
fn disable_changelog_disables_changelog_generation_for_packages() -> Result<()> {
    let mut settings = Settings {
        disable_changelog: true,
        ..Default::default()
    };
    init_monorepo(&mut settings)?;

    cog_bump(["--auto", "--package", "one"])?.success();

    assert_that!(Path::new("CHANGELOG.md")).does_not_exist();
    assert_tag_exists("one-0.1.0")?;
    Ok(())
}

// ##################
//     Skip untracked
// ##################

#[sealed_test]
fn uncommitted_changes_should_throw_error_by_default() -> Result<()> {
    init_monorepo(&mut Settings::default())?;

    run_cmd!(
        echo two > two;
    )?;

    cog_bump(["--auto", "--dry-run"])?.failure();

    Ok(())
}

#[sealed_test]
fn uncommitted_changes_should_not_throw_error_with_option() -> Result<()> {
    let mut settings = Settings {
        skip_untracked: true,
        ..Default::default()
    };

    init_monorepo(&mut settings)?;

    run_cmd!(
        echo two > two;
        echo "other changes" > one/file;
    )?;

    cog_bump(["--auto", "--dry-run"])?.success()
        .stderr("Untracked files :\n\tmodified: one/file\n\tnew: two\n\nnothing added to commit but untracked files present (use \"git add\" to track)\n\n")
        .stdout("one-0.1.0\n0.1.0\n\n");

    assert_that!(Path::new("CHANGELOG.md")).does_not_exist();
    assert_tag_does_not_exist("1.1.0")?;
    Ok(())
}

// ##################
//     Skip CI
// ##################

#[sealed_test]
fn bump_package_with_default_skip_ci_ok() -> Result<()> {
    let mut settings = Settings {
        ..Default::default()
    };
    init_monorepo(&mut settings)?;

    cog_bump(["--auto", "--skip-ci", "--package", "one"])?.success();

    let commit_message = git_log_head_message()?;

    assert_tag_exists("one-0.1.0")?;
    assert!(commit_message.contains("[skip ci]"));

    Ok(())
}

#[sealed_test]
fn bump_package_with_cog_toml_defined_skip_ci_ok() -> Result<()> {
    let mut settings = Settings {
        skip_ci: String::from("[ci-skip]"),
        ..Default::default()
    };
    init_monorepo(&mut settings)?;

    cog_bump(["--auto", "--skip-ci", "--package", "one"])?.success();

    let commit_message = git_log_head_message()?;

    assert_tag_exists("one-0.1.0")?;
    assert!(commit_message.contains("[ci-skip]"));

    Ok(())
}

#[sealed_test]
fn bump_package_with_skip_ci_override_option_takes_precedence() -> Result<()> {
    let mut settings = Settings {
        skip_ci: String::from("[ci-skip]"),
        ..Default::default()
    };
    init_monorepo(&mut settings)?;

    cog_bump([
        "--auto",
        "--skip-ci-override",
        "[ci-skip-override]",
        "--package",
        "one",
    ])?
    .success();

    let commit_message = git_log_head_message()?;

    assert_tag_exists("one-0.1.0")?;
    assert!(commit_message.contains("[ci-skip-override]"));

    Ok(())
}

#[sealed_test]
fn bump_standard_repository_with_default_skip_ci_ok() -> Result<()> {
    git_init()?;
    git_commit("chore: init")?;
    git_commit("feat: feature")?;

    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--skip-ci")
        .arg("--auto")
        .assert()
        .success();

    let commit_message = git_log_head_message()?;
    assert_tag_exists("0.1.0")?;
    assert!(commit_message.contains("[skip ci]"));

    Ok(())
}

#[sealed_test]
fn bump_standard_repository_with_cog_toml_defined_skip_ci_ok() -> Result<()> {
    git_init()?;
    git_commit("chore: init")?;
    git_add("skip_ci = \"[ci-skip]\"", "cog.toml")?;
    git_commit("feat: feature")?;

    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--skip-ci")
        .arg("--auto")
        .assert()
        .success();

    let commit_message = git_log_head_message()?;
    assert_tag_exists("0.1.0")?;
    assert!(commit_message.contains("[ci-skip]"));

    Ok(())
}

#[sealed_test]
fn bump_standard_repository_skip_ci_override_option_takes_precedence() -> Result<()> {
    git_init()?;
    git_commit("chore: init")?;
    git_add("skip_ci = \"[ci-skip]\"", "cog.toml")?;
    git_commit("feat: feature")?;

    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--skip-ci-override")
        .arg("[ci-skip-override]")
        .arg("--auto")
        .assert()
        .success();

    let commit_message = git_log_head_message()?;
    assert_tag_exists("0.1.0")?;
    assert!(commit_message.contains("[ci-skip-override]"));

    Ok(())
}

#[sealed_test]
fn bump_monorepo_with_default_skip_ci_ok() -> Result<()> {
    let mut settings = Settings {
        ..Default::default()
    };
    init_monorepo(&mut settings)?;

    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .arg("--skip-ci")
        .assert()
        .success();

    let commit_message = git_log_head_message()?;

    assert_tag_exists("0.1.0")?;
    assert!(commit_message.contains("[skip ci]"));

    Ok(())
}

#[sealed_test]
fn bump_monorepo_manual_increment_with_default_skip_ci_ok() -> Result<()> {
    let mut settings = Settings {
        ..Default::default()
    };
    init_monorepo(&mut settings)?;

    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--minor")
        .arg("--skip-ci")
        .assert()
        .success();

    let commit_message = git_log_head_message()?;

    assert_tag_exists("0.1.0")?;
    assert!(commit_message.contains("[skip ci]"));

    Ok(())
}

#[sealed_test]
fn bump_monorepo_with_cog_toml_defined_skip_ci_ok() -> Result<()> {
    let mut settings = Settings {
        skip_ci: String::from("[ci-skip]"),
        ..Default::default()
    };
    init_monorepo(&mut settings)?;

    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .arg("--skip-ci")
        .assert()
        .success();

    let commit_message = git_log_head_message()?;

    assert_tag_exists("0.1.0")?;
    assert!(commit_message.contains("[ci-skip]"));

    Ok(())
}

#[sealed_test]
fn bump_monorepo_skip_ci_override_option_takes_precedence() -> Result<()> {
    let mut settings = Settings {
        skip_ci: String::from("[ci-skip]"),
        ..Default::default()
    };

    init_monorepo(&mut settings)?;

    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .arg("--skip-ci-override")
        .arg("[skip-ci-override]")
        .assert()
        .success();

    let commit_message = git_log_head_message()?;

    assert_tag_exists("0.1.0")?;
    assert!(commit_message.contains("[skip-ci-override]"));

    Ok(())
}

#[sealed_test]
fn bump_only_package_with_default_skip_ci_ok() -> Result<()> {
    let mut settings = Settings {
        generate_mono_repository_global_tag: false,
        ..Default::default()
    };
    init_monorepo(&mut settings)?;

    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .arg("--skip-ci")
        .assert()
        .success();

    let commit_message = git_log_head_message()?;

    assert_tag_does_not_exist("0.1.0")?;
    assert_tag_exists("one-0.1.0")?;
    assert!(commit_message.contains("[skip ci]"));

    Ok(())
}

// ##################
//     No Bump Commit
// ##################

#[sealed_test]
fn disable_commit_creation_with_config_standard_ok() -> Result<()> {
    git_init()?;

    git_add("disable_bump_commit = true", "cog.toml")?;

    git_commit("chore: init")?;
    git_commit("feat: feature")?;

    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .assert()
        .success();

    assert_tag_exists("0.1.0")?;

    Command::new("git")
        .arg("status")
        .arg("-s")
        .assert()
        .success()
        .stdout(indoc!("A  CHANGELOG.md\n"));

    Ok(())
}

#[sealed_test]
fn disable_commit_creation_with_flag_standard_ok() -> Result<()> {
    git_init()?;

    git_commit("chore: init")?;
    git_commit("feat: feature")?;

    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .arg("--disable-bump-commit")
        .assert()
        .success();

    assert_tag_exists("0.1.0")?;

    Command::new("git")
        .arg("status")
        .arg("-s")
        .assert()
        .success()
        .stdout(indoc!("A  CHANGELOG.md\n"));

    Ok(())
}

#[sealed_test]
fn disable_commit_creation_with_pre_bump_hooks_standard_ok() -> Result<()> {
    git_init()?;

    git_add(
        "pre_bump_hooks = [\"echo pre_bump_file > pre_bump_file\"]\ndisable_bump_commit = true",
        "cog.toml",
    )?;

    git_commit("chore: init")?;
    git_commit("feat: feature")?;

    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .arg("--disable-bump-commit")
        .assert()
        .success();

    assert_tag_exists("0.1.0")?;

    Command::new("git")
        .arg("status")
        .arg("-s")
        .assert()
        .success()
        .stdout(indoc!("A  CHANGELOG.md\nA  pre_bump_file\n"));

    Ok(())
}

#[sealed_test]
fn disable_commit_creation_monorepo_ok() -> Result<()> {
    let mut settings = Settings {
        disable_bump_commit: true,
        ..Default::default()
    };
    init_monorepo(&mut settings)?;

    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .assert()
        .success();

    assert_tag_exists("0.1.0")?;
    assert_tag_exists("one-0.1.0")?;

    Command::new("git")
        .arg("status")
        .arg("-s")
        .assert()
        .success()
        .stdout(indoc!("A  CHANGELOG.md\nA  one/CHANGELOG.md\n"));

    Ok(())
}

#[sealed_test]
fn disable_commit_creation_package_ok() -> Result<()> {
    let mut settings = Settings {
        disable_bump_commit: true,
        ..Default::default()
    };
    init_monorepo(&mut settings)?;

    Command::cargo_bin("cog")?
        .arg("bump")
        .arg("--auto")
        .arg("--package")
        .arg("one")
        .arg("--disable-bump-commit")
        .assert()
        .success();

    assert_tag_exists("one-0.1.0")?;

    Command::new("git")
        .arg("status")
        .arg("-s")
        .assert()
        .success()
        .stdout(indoc!("A  one/CHANGELOG.md\n"));

    Ok(())
}
