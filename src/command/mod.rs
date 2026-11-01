pub mod bump;
pub mod changelog;
pub mod check;
pub mod commit;
pub mod edit;
pub mod get_version;
pub mod git_hooks;
pub mod init;
pub mod log;

/// Command Targets
///
/// Targets can be:
/// * `standard` - entire repository without distinguishing between packages
/// * `global` - everything outside of any package
/// * `package` - single package
pub struct Targets {
    pub standard: bool,
    pub global: bool,
    pub packages: Vec<String>,
}
