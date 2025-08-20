use std::cmp::Ordering;

use semver::Version;

#[derive(Debug, PartialEq, Eq, Default, Clone)]
pub enum IncrementCommand {
    #[default]
    Auto,
    Fixed(Increment),
    AutoMonoRepoGlobal(Option<Increment>),
    Manual(String),
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum Increment {
    Major,
    Minor,
    Patch,
    NoBump,
}

impl Increment {
    pub fn add_to_version(&self, version: &mut Version) {
        match self {
            Increment::Major => {
                version.major += 1;
                version.minor = 0;
                version.patch = 0;
            }
            Increment::Minor => {
                version.minor += 1;
                version.patch = 0;
            }
            Increment::Patch => {
                version.patch += 1;
            }
            Increment::NoBump => {}
        }
    }
}

impl Ord for Increment {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (increment, other) if increment == other => Ordering::Equal,
            (Increment::Major, _) => Ordering::Greater,
            (_, Increment::Major) => Ordering::Less,
            (Increment::Minor, _) => Ordering::Greater,
            (_, Increment::Minor) => Ordering::Less,
            (Increment::Patch, Increment::Patch) => Ordering::Equal,
            (Increment::NoBump, Increment::NoBump) => Ordering::Equal,
            (Increment::Patch, Increment::NoBump) => Ordering::Greater,
            (Increment::NoBump, Increment::Patch) => Ordering::Less,
        }
    }
}

impl PartialOrd for Increment {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
// Auto version tests resides in test/ dir since it rely on git log
// To generate the version
mod test {
    // TODO
}
