use crate::conventional::commit::Commit;
use crate::error::CogCheckReport;

use crate::git::rev::revspec::RevSpec;
use crate::git::tag::TagLookUpOptions;
use crate::CocoGitto;
use anyhow::anyhow;
use anyhow::Result;
use colored::*;
use log::info;

impl CocoGitto {
    pub fn check(
        &self,
        check_from_latest_tag: bool,
        ignore_merge_commits: bool,
        ignore_fixup_commits: bool,
        range: Option<String>,
    ) -> Result<()> {
        let spec = if let Some(range) = range {
            RevSpec::parse(&range, &self.repository)?
        } else if check_from_latest_tag {
            let tag = self
                .repository
                .get_latest_tag(TagLookUpOptions::default())?;
            RevSpec::from_tag(&tag)
        } else {
            RevSpec::default()
        };

        let mut commit_range = self.repository.revwalk(&spec)?;
        commit_range.filter_merge_fixup_commits(ignore_merge_commits, ignore_fixup_commits);

        let errors: Vec<_> = commit_range
            .commits
            .iter()
            .filter_map(|(commit, _)| Commit::from_git_commit(commit).err())
            .collect();

        if errors.is_empty() {
            let msg = "No errored commits".green();
            info!("{}", msg);
            Ok(())
        } else {
            let report = CogCheckReport {
                from: commit_range.from(),
                errors: errors.into_iter().map(|err| *err).collect(),
            };
            Err(anyhow!("{}", report))
        }
    }
}
