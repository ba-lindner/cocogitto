use anyhow::bail;
use anyhow::Result;
use log::warn;
use semver::Version;

use crate::git::error::TagError;
use crate::CocoGitto;

impl CocoGitto {
    pub fn get_latest_version(
        &self,
        fallback: Option<String>,
        package: Option<String>,
        include_prereleases: bool,
        print_tag: bool,
    ) -> Result<()> {
        let fallback = match fallback {
            Some(input) => match Version::parse(&input) {
                Ok(version) => Some(version),
                Err(err) => {
                    warn!("Invalid fallback: {}", input);
                    bail!("{}", err)
                }
            },
            None => None,
        };

        let current_tag = self
            .repository
            .get_latest_tag(package.as_deref(), include_prereleases);

        let current_version = match current_tag {
            Ok(tag) => {
                if print_tag {
                    tag.to_string()
                } else {
                    tag.version.to_string()
                }
            }
            Err(TagError::NoTag) => match fallback {
                Some(version) => version.to_string(),
                None => bail!("No version yet"),
            },
            Err(err) => bail!("{}", err),
        };

        warn!("Current version:");
        print!("{current_version}");
        Ok(())
    }
}
