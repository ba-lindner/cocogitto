//! Tests for bump hooks

use super::*;

#[sealed_test]
#[cfg(target_os = "linux")]
fn bump_with_hook() -> Result<()> {
    // Arrange
    git_init()?;
    git_add(r#"pre_bump_hooks = ["touch {{version}}"]"#, "cog.toml")?;
    git_commit("chore: init")?;
    git_tag("1.0.0")?;
    git_commit("feat: feature")?;

    // Act
    cog_bump(["--major"])?.success();

    // Assert
    assert_that!(Path::new("2.0.0")).exists();
    assert_tag_exists("2.0.0")?;
    Ok(())
}

#[sealed_test]
#[cfg(target_os = "linux")]
fn bump_with_profile_hook() -> Result<()> {
    // Arrange
    git_init()?;

    let config = indoc! {
        "[bump_profiles.custom]
        pre_bump_hooks = [\"echo {{latest}} > version\"]
        post_bump_hooks = [\"echo {{version}} >> version\"]
        "
    };

    git_add(config, "cog.toml")?;

    git_commit("chore: init")?;
    git_tag("1.0.0")?;
    git_commit("feat: feature")?;

    // Act
    cog_bump(["--patch", "--hook-profile", "custom"])?.success();

    // Assert
    assert_tag_exists("1.0.1")?;
    assert_that!(std::fs::read_to_string("version"))
        .is_ok()
        .is_equal_to("1.0.0\n1.0.1\n".to_string());
    Ok(())
}
