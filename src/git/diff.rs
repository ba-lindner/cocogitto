use crate::git::repository::Repository;
use git2::DiffOptions;

impl Repository {
    pub(crate) fn is_dirty(&self, include_untracked: bool) -> bool {
        let mut options = DiffOptions::new();
        options.include_untracked(include_untracked);

        let head = self.0.head().and_then(|head| head.peel_to_tree());

        let diff = if let Ok(tree) = head {
            self.0
                .diff_tree_to_index(Some(&tree), None, Some(&mut options))
        } else {
            self.0
                .diff_tree_to_workdir_with_index(None, Some(&mut options))
        };

        diff.is_ok_and(|diff| diff.deltas().len() > 0)
    }
}

#[cfg(test)]
mod test {
    use crate::test_helpers::git_init_no_gpg;
    use anyhow::Result;
    use cmd_lib::run_cmd;
    use sealed_test::prelude::*;
    use speculoos::prelude::*;

    #[sealed_test]
    fn get_diff_some() -> Result<()> {
        let repo = git_init_no_gpg()?;

        // Arrange
        run_cmd!(
            echo changes > file;
            git add .;
        )?;

        // Act
        let dirty = repo.is_dirty(false);

        // Assert
        assert_that!(dirty).is_true();
        Ok(())
    }

    #[sealed_test]
    fn get_diff_none() -> Result<()> {
        let repo = git_init_no_gpg()?;

        // Arrange
        run_cmd!(
            echo changes > file;
        )?;

        // Act
        let dirty = repo.is_dirty(false);

        // Assert
        assert_that!(dirty).is_false();
        Ok(())
    }

    #[sealed_test]
    fn get_diff_include_untracked_some() -> Result<()> {
        let repo = git_init_no_gpg()?;

        // Arrange
        run_cmd!(
            echo changes > file;
        )?;

        // Act
        let dirty = repo.is_dirty(true);

        // Assert
        assert_that!(dirty).is_true();
        Ok(())
    }

    #[sealed_test]
    fn get_diff_include_untracked_none() -> Result<()> {
        // Arrange
        let repo = git_init_no_gpg()?;

        // Act
        let dirty = repo.is_dirty(true);

        // Assert
        assert_that!(dirty).is_false();
        Ok(())
    }
}
