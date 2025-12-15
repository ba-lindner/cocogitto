use anyhow::{ensure, Result};
use semver::Version;

use crate::git::tag::Tag;

/**
 * Increments a pre-release version based on a pattern.
 *
 * For example, given "alpha.2" and the pattern "alpha.*", it increments to "alpha.3".
 *
 * If the pre-release version doesn't match the pattern, it starts a new sequence based on the
 * pattern.
 *
 * For example, given "alpha.2" and the pattern "beta.*", it returns "beta.1".
 *
 * If the next version is greater than the current pre-release expected, it starts a new sequence.
 * For example, if the current pre-release is 1.2.3-alpha.1 and the next version is 2.0.0, then the
 * next pre-release would be 2.0.0-alpha.1 (not 2.0.0-alpha.2).
 *
 * - current_prerelease The latest pre-release version
 * - next The next version
 * - pattern The exact pre-release label or pattern, for example "alpha.1" or "alpha.*"
 * */
pub(super) fn increment_prerelease(
    current_prerelease: &Option<Tag>,
    next: &Tag,
    prerelease: &str,
) -> Result<String> {
    ensure!(!prerelease.is_empty(), "Pre-release can't be empty");

    if let Some(current_prerelease) = current_prerelease {
        // Ensure the current pre-release is less than next version
        ensure!(
            current_prerelease.version < next.version,
            "Current pre-release ({}) is greater than the next version ({})",
            current_prerelease,
            next
        );
    }

    // Validate the pattern contains just one asterisk ("*")
    let pattern_parts: Vec<&str> = prerelease.split('*').collect();
    ensure!(
        pattern_parts.len() == 2,
        "Pre-release pattern ({}) is invalid. Valid patterns contain exactly one wildcard (*)",
        prerelease
    );

    let (prefix, suffix) = (pattern_parts[0], pattern_parts[1]);
    let initial_prerelease = format!("{}1{}", prefix, suffix);

    let new_prerelease = current_prerelease
        .as_ref()
        .and_then(|current| current.increment_pre_by_pattern(prefix, suffix, next))
        .unwrap_or(initial_prerelease);
    Ok(new_prerelease)
}

impl Tag {
    fn increment_pre_by_pattern(&self, prefix: &str, suffix: &str, next: &Tag) -> Option<String> {
        // Check if the current pre-release was expecting a different version to what the next pre-release expects
        if self.core_version() != next.core_version() {
            return None;
        }

        let pre_to_increment = self.version.pre.as_str();
        // Try to extract the numeric identifier
        // Also checks if the current pre-release matches the pattern format
        let num: u64 = pre_to_increment
            .strip_prefix(prefix)?
            .strip_suffix(suffix)?
            .parse()
            .ok()?;
        Some(format!("{prefix}{}{suffix}", num + 1))
    }

    fn core_version(&self) -> Version {
        Version::new(self.version.major, self.version.minor, self.version.patch)
    }
}

#[cfg(test)]
mod test {
    use anyhow::Result;

    use crate::{command::bump::increment_prerelease, git::tag::Tag};

    #[test]
    fn increment_prerelease_invalid_empty() -> Result<()> {
        let current_prerelease = Option::None::<Tag>;
        let next = Tag::from_str("1.2.3", None)?;
        let prerelease = "";

        let result = increment_prerelease(&current_prerelease, &next, prerelease);

        assert!(result.is_err());
        assert_eq!(
            "Pre-release can't be empty",
            result.unwrap_err().to_string()
        );

        Ok(())
    }

    #[test]
    fn increment_prerelease_invalid_no_pattern() -> Result<()> {
        let current_prerelease = Option::None::<Tag>;
        let next = Tag::from_str("1.2.3", None)?;
        let prerelease = "alpha.4";

        let result = increment_prerelease(&current_prerelease, &next, prerelease);

        assert!(result.is_err());
        assert_eq!(
            "Pre-release pattern (alpha.4) is invalid. Valid patterns contain exactly one wildcard (*)",
            result.unwrap_err().to_string()
        );
        Ok(())
    }

    #[test]
    fn increment_prerelease_invalid_pattern() -> Result<()> {
        let current_prerelease = Option::None::<Tag>;
        let next = Tag::from_str("1.2.3", None)?;
        let prerelease = "alpha.**";

        let result = increment_prerelease(&current_prerelease, &next, prerelease);

        assert!(result.is_err());
        assert_eq!(
            "Pre-release pattern (alpha.**) is invalid. Valid patterns contain exactly one wildcard (*)",
            result.unwrap_err().to_string()
        );

        Ok(())
    }

    #[test]
    fn increment_prerelease_invalid_pattern_2() -> Result<()> {
        let current_prerelease = Option::None::<Tag>;
        let next = Tag::from_str("1.2.3", None)?;
        let prerelease = "**";

        let result = increment_prerelease(&current_prerelease, &next, prerelease);

        assert!(result.is_err());
        assert_eq!(
            "Pre-release pattern (**) is invalid. Valid patterns contain exactly one wildcard (*)",
            result.unwrap_err().to_string()
        );

        Ok(())
    }

    #[test]
    fn increment_prerelease_invalid_pattern_3() -> Result<()> {
        let current_prerelease = Option::None::<Tag>;
        let next = Tag::from_str("1.2.3", None)?;
        let prerelease = "***";

        let result = increment_prerelease(&current_prerelease, &next, prerelease);

        assert!(result.is_err());
        assert_eq!(
            "Pre-release pattern (***) is invalid. Valid patterns contain exactly one wildcard (*)",
            result.unwrap_err().to_string()
        );

        Ok(())
    }

    #[test]
    fn increment_prerelease_initial() -> Result<()> {
        let current_prerelease = Option::None::<Tag>;
        let next = Tag::from_str("1.2.3", None)?;
        let prerelease = "alpha.*";

        let result = increment_prerelease(&current_prerelease, &next, prerelease)?;

        assert_eq!("alpha.1", result);
        Ok(())
    }

    #[test]
    fn increment_prerelease_initial_no_match() -> Result<()> {
        let current_prerelease = Tag::from_str("1.2.3-alpha.4", None)?;
        let next = Tag::from_str("1.2.3", None)?;
        let prerelease = "beta.*";

        let result = increment_prerelease(&Some(current_prerelease), &next, prerelease)?;

        assert_eq!("beta.1", result);
        Ok(())
    }

    #[test]
    fn increment_prerelease_partial_match() -> Result<()> {
        let current_prerelease = Tag::from_str("1.2.3-alpha.preview.1", None)?;
        let next = Tag::from_str("1.2.3", None)?;
        let prerelease = "alpha.*";

        let result = increment_prerelease(&Some(current_prerelease), &next, prerelease)?;

        assert_eq!("alpha.1", result);
        Ok(())
    }

    #[test]
    fn increment_prerelease_initial_after_prerelease() -> Result<()> {
        let current_prerelease = Some(Tag::from_str("1.2.3-alpha.5", None)?);
        let next = Tag::from_str("1.3.0", None)?;
        let prerelease = "alpha.*";

        let result = increment_prerelease(&current_prerelease, &next, prerelease)?;

        assert_eq!("alpha.1", result);
        Ok(())
    }

    #[test]
    fn increment_prerelease_invalid_after_later_prerelease() -> Result<()> {
        let current_prerelease = Some(Tag::from_str("1.3.0-alpha.5", None)?);
        let next = Tag::from_str("1.2.0", None)?;
        let prerelease = "alpha.*";

        let result = increment_prerelease(&current_prerelease, &next, prerelease);

        assert!(result.is_err());
        assert_eq!(
            "Current pre-release (1.3.0-alpha.5) is greater than the next version (1.2.0)",
            result.unwrap_err().to_string()
        );

        Ok(())
    }

    #[test]
    fn increment_prerelease_invalid_after_later_prerelease_and_no_pattern() -> Result<()> {
        let current_prerelease = Some(Tag::from_str("1.3.0-alpha.5", None)?);
        let next = Tag::from_str("1.2.0", None)?;
        let prerelease = "alpha.4";

        let result = increment_prerelease(&current_prerelease, &next, prerelease);

        assert!(result.is_err());
        assert_eq!(
            "Current pre-release (1.3.0-alpha.5) is greater than the next version (1.2.0)",
            result.unwrap_err().to_string()
        );

        Ok(())
    }

    #[test]
    fn increment_prerelease_wildcard_at_start() -> Result<()> {
        let current_prerelease = Tag::from_str("1.2.3-4.alpha", None)?;
        let next = Tag::from_str("1.2.3", None)?;
        let prerelease = "*.alpha";

        let result = increment_prerelease(&Some(current_prerelease), &next, prerelease)?;

        assert_eq!("5.alpha", result);
        Ok(())
    }

    #[test]
    fn increment_prerelease_wildcard_in_middle() -> Result<()> {
        let current_prerelease = Tag::from_str("1.2.3-alpha.4.fix", None)?;
        let next = Tag::from_str("1.2.3", None)?;
        let prerelease = "alpha.*.fix";

        let result = increment_prerelease(&Some(current_prerelease), &next, prerelease)?;

        assert_eq!("alpha.5.fix", result);
        Ok(())
    }

    #[test]
    fn increment_prerelease_wildcard_only() -> Result<()> {
        let current_prerelease = Tag::from_str("1.2.3-4", None)?;
        let next = Tag::from_str("1.2.3", None)?;
        let prerelease = "*";

        let result = increment_prerelease(&Some(current_prerelease), &next, prerelease)?;

        assert_eq!("5", result);
        Ok(())
    }

    #[test]
    fn increment_prerelease_wildcard_at_end() -> Result<()> {
        let current_prerelease = Tag::from_str("1.2.3-alpha.4", None)?;
        let next = Tag::from_str("1.2.3", None)?;
        let prerelease = "alpha.*";

        let result = increment_prerelease(&Some(current_prerelease), &next, prerelease)?;

        assert_eq!("alpha.5", result);
        Ok(())
    }
}
