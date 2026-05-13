use crate::conventional::commit::Commit;
use crate::error::CogCheckReport;

use crate::git::rev::revspec::RevSpecPattern2;
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
        let pattern = if let Some(range) = range {
            self.repository.revspec_from_str(&range)?
        } else if check_from_latest_tag {
            let tag = self
                .repository
                .get_latest_tag(TagLookUpOptions::default())?;
            RevSpecPattern2::from(tag.oid.expect("latest tag should have oid"))
        } else {
            RevSpecPattern2::full()
        };
        let commit_range = self
            .repository
            .revwalk(pattern)?
            .ignore_commits(ignore_merge_commits, ignore_fixup_commits);

        let errors: Vec<_> = commit_range
            .iter_commits()
            .map(Commit::from_git_commit)
            .filter_map(Result::err)
            .collect();

        if errors.is_empty() {
            let msg = "No errored commits".green();
            info!("{}", msg);
            Ok(())
        } else {
            let report = CogCheckReport {
                from: commit_range.from_oid(),
                errors: errors.into_iter().map(|err| *err).collect(),
            };
            Err(anyhow!("{}", report))
        }
    }
}
