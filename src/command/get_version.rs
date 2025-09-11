use anyhow::anyhow;
use anyhow::bail;
use anyhow::Result;
use log::warn;
use semver::Version;

use crate::CocoGitto;

impl CocoGitto {
    pub fn get_latest_version(
        &self,
        fallback: Option<String>,
        package: Option<String>,
        include_pre_release: bool,
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

        let current_version = self
            .repository
            .get_latest_tag(package.as_deref(), include_pre_release)
            .map(|tag| tag.version)
            .or(fallback)
            .ok_or(anyhow!("No version yet"))?;

        warn!("Current version:");
        print!("{current_version}");
        Ok(())
    }
}
