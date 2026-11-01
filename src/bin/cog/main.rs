mod commit;
mod mangen;

use std::fs;
use std::path::PathBuf;

use cocogitto::command::Targets;
use cocogitto::conventional::changelog::template::{RemoteContext, Template};
use cocogitto::conventional::changelog::ReleaseType;
use cocogitto::conventional::commit as conv_commit;
use cocogitto::conventional::version::IncrementCommand;

use cocogitto::log::filter::{CommitFilter, CommitFilters};
use cocogitto::log::output::Output;
use cocogitto::{set_config_path, CocoGitto, CommitHook, DEFAULT_CONFIG_PATH, SETTINGS};

use crate::commit::prepare_edit_message;
use anyhow::{anyhow, bail, Context, Result};
use clap::builder::{PossibleValue, PossibleValuesParser};
use clap::{ArgAction, ArgGroup, Args, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{shells, Generator};
use clap_complete_nushell::Nushell;
use cocogitto::command::bump::{BumpOptions, PackageBumpOptions};
use cocogitto::command::commit::CommitOptions;
use cocogitto::settings::{GitHookType, MonoRepoPackage};

fn hook_profiles() -> PossibleValuesParser {
    let profiles = SETTINGS
        .bump_profiles
        .keys()
        .map(|profile| -> &str { profile });

    profiles.into()
}

fn git_hook_types() -> PossibleValuesParser {
    let hooks = SETTINGS
        .git_hooks
        .keys()
        .map(|hook_type| hook_type.to_string());

    hooks.into()
}

fn packages() -> PossibleValuesParser {
    let profiles = SETTINGS.packages.keys().map(|profile| -> &str { profile });

    profiles.into()
}

/// Shell with auto-generated completion script available.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Shell {
    /// Bourne Again SHell (bash)
    Bash,
    /// Elvish shell
    Elvish,
    /// Friendly Interactive SHell (fish)
    Fish,
    /// PowerShell
    PowerShell,
    /// Z SHell (zsh)
    Zsh,
    /// Nu shell (nu)
    Nu,
}

impl Generator for Shell {
    fn file_name(&self, name: &str) -> String {
        match self {
            Shell::Bash => shells::Bash.file_name(name),
            Shell::Elvish => shells::Elvish.file_name(name),
            Shell::Fish => shells::Fish.file_name(name),
            Shell::PowerShell => shells::PowerShell.file_name(name),
            Shell::Zsh => shells::Zsh.file_name(name),
            Shell::Nu => Nushell.file_name(name),
        }
    }

    fn generate(&self, cmd: &clap::Command, buf: &mut dyn std::io::Write) {
        match self {
            Shell::Bash => shells::Bash.generate(cmd, buf),
            Shell::Elvish => shells::Elvish.generate(cmd, buf),
            Shell::Fish => shells::Fish.generate(cmd, buf),
            Shell::PowerShell => shells::PowerShell.generate(cmd, buf),
            Shell::Zsh => shells::Zsh.generate(cmd, buf),
            Shell::Nu => Nushell.generate(cmd, buf),
        }
    }
}

// Hand-rolled so it can work even when `derive` feature is disabled
impl ValueEnum for Shell {
    fn value_variants<'a>() -> &'a [Self] {
        &[
            Shell::Bash,
            Shell::Elvish,
            Shell::Fish,
            Shell::PowerShell,
            Shell::Zsh,
            Shell::Nu,
        ]
    }

    fn to_possible_value<'a>(&self) -> Option<PossibleValue> {
        Some(match self {
            Shell::Bash => PossibleValue::new("bash"),
            Shell::Elvish => PossibleValue::new("elvish"),
            Shell::Fish => PossibleValue::new("fish"),
            Shell::PowerShell => PossibleValue::new("powershell"),
            Shell::Zsh => PossibleValue::new("zsh"),
            Shell::Nu => PossibleValue::new("nu"),
        })
    }
}

/// A command line tool for the conventional commits and semver specifications
#[derive(Parser)]
#[command(
    version,
    name = "cog",
    author = "Paul D. <paul.delafosse@protonmail.com>"
)]
struct Cli {
    /// The level of verbosity: -v for ERROR, -vv for WARNING, -vvv for INFO
    #[arg(long, short = 'v', action = ArgAction::Count)]
    verbose: u8,

    /// Silence all output, no matter the value of verbosity
    #[arg(long, short = 'q')]
    quiet: bool,

    // NOTE: just here to show up in help, not used directly
    /// Path to config file
    #[arg(long = "config", default_value = DEFAULT_CONFIG_PATH)]
    config: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Verify all commit messages against the conventional commit specification
    Check {
        /// Check commit history, starting from the latest tag to HEAD
        #[arg(short = 'l', long, group = "commit_range")]
        from_latest_tag: bool,

        /// Ignore merge commits messages
        #[arg(short, long)]
        ignore_merge_commits: bool,

        /// Ignore fixup!, squash! and amend! commit messages
        #[arg(long)]
        ignore_fixup_commits: bool,

        /// Check commits in the specified range
        #[arg(group = "commit_range")]
        range: Option<String>,
    },

    /// Create a new conventional commit
    Commit(CommitArgs),

    /// Interactively rename invalid commit messages
    Edit {
        /// Edit non conventional commits, starting from the latest tag to HEAD
        #[arg(short = 'l', long)]
        from_latest_tag: bool,
    },

    /// Like git log but for conventional commits
    Log {
        /// Filter BREAKING CHANGE commits
        #[arg(short = 'B', long)]
        breaking_change: bool,

        /// Filter on commit type
        #[arg(short, long = "type", value_name = "type")]
        typ: Option<Vec<String>>,

        /// Filter on commit author
        #[arg(short, long)]
        author: Option<Vec<String>>,

        /// Filter on commit scope
        #[arg(short, long)]
        scope: Option<Vec<String>>,

        /// Omit error on the commit log
        #[arg(short = 'e', long)]
        no_error: bool,
    },

    /// Verify a single commit message
    #[command(group = ArgGroup::new("verify_input").required(true))]
    Verify {
        /// The commit message
        #[arg(group = "verify_input")]
        message: Option<String>,

        /// Read message from the specified file
        #[arg(short, long, group = "verify_input")]
        file: Option<PathBuf>,

        /// Ignore merge commit messages
        #[arg(short, long)]
        ignore_merge_commits: bool,

        /// Ignore fixup!, squash! and amend! commit messages
        #[arg(long)]
        ignore_fixup_commits: bool,
    },

    /// Display a changelog for the given commit oid range
    Changelog {
        /// Generate the changelog in the given spec range
        #[arg(conflicts_with = "at")]
        pattern: Option<String>,

        /// Generate the changelog for a specific git tag
        #[arg(short, long)]
        at: Option<String>,

        /// Generate the changelog with the given template.
        ///
        /// Possible values are 'remote', 'full_hash', 'default' or the path to your template.
        /// If not specified cog will use cog.toml template config or fallback to 'default'.
        #[arg(long, short)]
        template: Option<String>,

        /// Url to use during template generation
        #[arg(long, short, requires_all = ["owner", "repository"])]
        remote: Option<String>,

        /// Repository owner to use during template generation
        #[arg(long, short, requires_all = ["remote", "repository"])]
        owner: Option<String>,

        /// Name of the repository used during template generation
        #[arg(long, requires_all = ["owner", "remote"])]
        repository: Option<String>,

        /// Combine package and global changes into one changelog
        #[arg(short, long)]
        unified: bool,
    },

    /// Get current version
    GetVersion {
        /// Fallback version. Has to be semver compliant.
        #[arg(short, long)]
        fallback: Option<String>,

        /// Specify which package to get the version for in a monorepo.
        #[arg(long, value_parser = packages())]
        package: Option<String>,

        /// Include prerelease versions
        #[arg(short, long)]
        include_prereleases: bool,

        /// Print full tag
        #[arg(short, long)]
        tag: bool,
    },

    /// Commit changelog from latest tag to HEAD and create new tag
    #[command(group = ArgGroup::new("bump-spec").required(true))]
    Bump {
        /// Manually set the target version
        #[arg(long, group = "bump-spec")]
        version: Option<String>,

        /// Automatically suggest the target version
        #[arg(short, long, group = "bump-spec")]
        auto: bool,

        /// Increment the major version
        #[arg(short = 'M', long, group = "bump-spec")]
        major: bool,

        /// Increment the minor version
        #[arg(short, long, group = "bump-spec")]
        minor: bool,

        /// Increment the patch version
        #[arg(short, long, group = "bump-spec")]
        patch: bool,

        /// Set the pre-release version
        #[arg(long)]
        pre: Option<String>,

        /// Set the build suffix
        #[arg(long)]
        build: Option<String>,

        /// Specify the bump profile hooks to run
        #[arg(short = 'H', long, value_parser = hook_profiles())]
        hook_profile: Option<String>,

        /// Specify which package to bump for monorepo
        #[arg(long, value_parser = packages())]
        package: Option<String>,

        /// Annotate tag with given message
        #[arg(short = 'A', long)]
        annotated: Option<String>,

        /// Dry-run: print the target version. No action taken
        #[arg(short, long)]
        dry_run: bool,

        /// Add the skip-ci string defined in the cog.toml (or defaults to [skip ci]) to the bump commit
        #[arg(long = "skip-ci")]
        skip_ci: bool,

        /// Override and add the skip-ci string with the provided value to the bump commit
        #[arg(long = "skip-ci-override")]
        skip_ci_override: Option<String>,

        /// Don't fail if there are untracked or uncommitted files
        #[arg(long = "skip-untracked")]
        skip_untracked: bool,

        /// Disable the creation of the bump commit
        #[arg(long = "disable-bump-commit")]
        disable_bump_commit: bool,

        /// Also bump packages on manual bump
        ///
        /// Overrides the default behaviour for --patch, --minor, --major and --version to only
        /// bump the global version for monorepos. Useful to bump to version 1.0.0.
        #[arg(long, conflicts_with = "auto")]
        include_packages: bool,
    },

    /// Install cog config files
    Init {
        /// Path to initialized dir
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Add git hooks to the repository
    InstallHook {
        /// Type of hook to install
        #[arg(value_parser = git_hook_types(),  group = "git-hooks")]
        hook_type: Vec<String>,
        /// Install all git-hooks
        #[arg(short, long, group = "git-hooks")]
        all: bool,
        /// Overwrite existing git-hooks
        #[arg(short, long)]
        overwrite: bool,
    },

    /// Generate shell completions
    GenerateCompletions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Generate manpage
    #[command(hide = true)]
    GenerateManpages {
        output_dir: PathBuf,
    },

    Target(#[command(flatten)] TargetArgs),
}

#[derive(Args)]
struct CommitArgs {
    /// Conventional commit type
    #[arg(name = "type", value_name = "TYPE", value_parser = commit::commit_types())]
    typ: String,

    /// Commit description
    message: String,

    /// Conventional commit scope
    scope: Option<String>,

    /// Create a BREAKING CHANGE commit
    #[arg(short = 'B', long)]
    breaking_change: bool,

    /// Open commit message in an editor
    #[arg(short, long)]
    edit: bool,

    /// Sign this commit
    #[arg(short, long)]
    sign: bool,

    /// Add the skip-ci string defined in the cog.toml (or defaults to [skip ci]) to the commit
    #[arg(long = "skip-ci")]
    skip_ci: bool,

    /// Override and add the <SKIP_CI_OVERRIDE> string to the commit
    #[arg(long = "skip-ci-override")]
    skip_ci_override: Option<String>,

    /// Add files to the commit (similar to git add .)
    #[arg(short, long = "add")]
    add_files: bool,

    /// Update but doesn't add files to the commit (similar to git add -u .)
    #[arg(short, long = "update")]
    update_files: bool,
}

/// Target selection
///
/// only available / useful in monorepos,
/// as otherwise `standard` is the only reasonable target
#[derive(Args)]
struct TargetArgs {
    /// Include global + all packages
    ///
    /// Default for bump --auto and changelog --unified
    #[arg(long, conflicts_with_all = ["global", "package", "all_packages", "packages_if"])]
    all_targets: bool,
    /// Global target
    ///
    /// Default for manual bumping, get-version and changelog
    #[arg(long)]
    global: bool,
    /// Single package
    #[arg(short, long)]
    package: Vec<String>,
    /// All packages
    #[arg(long, conflicts_with_all = ["package", "packages_if"])]
    all_packages: bool,
    /// Include packages that match the given condition
    ///
    /// A condition must be given as <field> <operator> <value>, e.g.
    /// `public_api=true`
    #[arg(long)]
    packages_if: Vec<String>,
    /// Exclude a package
    #[arg(long)]
    exclude: Vec<String>,
    /// Limit packages to those matching the given condition
    ///
    /// Same conditions as with `packages_if` can be used
    #[arg(long)]
    filter: Vec<String>,
}

impl TargetArgs {
    fn get_targets(self) -> Result<Targets> {
        // outside monorepo, only `standard` is a valid target
        if SETTINGS.packages.is_empty() {
            return Ok(Targets {
                standard: true,
                global: false,
                packages: Vec::new(),
            });
        }

        // `exclude` and `filter` don't prevent default
        let use_default = self.package.is_empty()
            && self.packages_if.is_empty()
            && !self.all_targets
            && !self.global
            && !self.all_packages;

        let mut res = if use_default {
            Targets {
                standard: false,
                global: true,
                packages: SETTINGS.packages.keys().cloned().collect(),
            }
        } else {
            Targets {
                standard: false,
                global: self.global || self.all_targets,
                packages: if self.all_packages || self.all_targets {
                    SETTINGS.packages.keys().cloned().collect()
                } else {
                    self.package
                },
            }
        };

        fn parse_filter(
            mut filter: &str,
        ) -> Result<Box<dyn Fn(&str, &MonoRepoPackage) -> bool + '_>> {
            type Operator = (
                fn(&str, &str) -> bool,
                Option<fn(&Option<usize>, &Option<usize>) -> bool>,
                Option<bool>,
            );
            const FIELDS: &[(
                &str,
                for<'a> fn(
                    Operator,
                    &'a str,
                )
                    -> Result<Box<dyn Fn(&str, &MonoRepoPackage) -> bool + 'a>>,
            )] = &[
                ("name", |op, val| {
                    Ok(Box::new(move |name, _| (op.0)(name, val)))
                }),
                ("path", |op, val| {
                    Ok(Box::new(move |_, pkg| {
                        (op.0)(&pkg.path.display().to_string(), val)
                    }))
                }),
                ("changelog_path", |op, val| {
                    Ok(Box::new(move |_, pkg| {
                        (op.0)(pkg.changelog_path.as_deref().unwrap_or_default(), val)
                    }))
                }),
                ("public_api", |op, val| {
                    let expected =
                        op.2.ok_or(anyhow!("operator not supported for public_api"))?
                            == val.parse::<bool>()?;
                    Ok(Box::new(move |_, pkg| pkg.public_api == expected))
                }),
                ("bump_order", |op, val| {
                    let val = if val.is_empty() {
                        None
                    } else {
                        Some(val.parse()?)
                    };
                    let cmp =
                        op.1.ok_or(anyhow!("operator not supported for bump_order"))?;
                    Ok(Box::new(move |_, pkg| cmp(&pkg.bump_order, &val)))
                }),
            ];
            const OPS: &[(&str, Operator)] = &[
                ("!~", (|a, b| !a.contains(b), None, None)),
                ("~", (|a, b| a.contains(b), None, None)),
                ("=", (str::eq, Some(Option::<usize>::eq), Some(true))),
                ("!=", (str::ne, Some(Option::<usize>::ne), Some(false))),
                ("<=", (str::le, Some(Option::<usize>::le), None)),
                ("<", (str::lt, Some(Option::<usize>::lt), None)),
                (">=", (str::ge, Some(Option::<usize>::ge), None)),
                (">", (str::gt, Some(Option::<usize>::gt), None)),
            ];
            filter = filter.trim_start();
            let &(field, field_func) =
                FIELDS
                    .iter()
                    .find(|(field, _)| filter.starts_with(field))
                    .ok_or(anyhow!("could not find field for package_filter {filter}"))?;
            filter = filter
                .strip_prefix(field)
                .expect("checked in loop")
                .trim_start();
            let &(op, op_func) = OPS
                .iter()
                .find(|(op, _)| filter.starts_with(op))
                .ok_or(anyhow!("could not find operator at beginning of {filter}"))?;
            filter = filter
                .strip_prefix(op)
                .expect("checked in loop")
                .trim_start();
            field_func(op_func, filter)
        }

        for filter in &self.packages_if {
            let filter = parse_filter(filter)?;
            for (pkg_name, package) in &SETTINGS.packages {
                if filter(pkg_name, package) {
                    res.packages.push(pkg_name.clone());
                }
            }
        }

        for filter in &self.filter {
            let filter = parse_filter(filter)?;
            res.packages
                .retain(|pkg| filter(pkg, &SETTINGS.packages[pkg]));
        }

        res.packages.retain(|pkg| !self.exclude.contains(pkg));

        res.packages.sort();
        Ok(res)
    }
}

/// Loads "--config" argument before loading the full CLI as other CLI commands
/// require the config file (to load possible values)
fn load_config_path() -> String {
    let config_parser = clap::Command::new("bootstrap")
        .disable_help_flag(true)
        .ignore_errors(true)
        .allow_external_subcommands(true)
        .arg(clap::Arg::new("config").long("config").required(false));
    let matches = config_parser.get_matches();

    let config_path = matches
        .get_one::<String>("config")
        .map(|s| s.to_owned())
        .unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_owned());

    config_path
}

fn main() -> Result<()> {
    let config_path = load_config_path();
    set_config_path(config_path);

    let cli = Cli::parse();

    init_logs(cli.verbose, cli.quiet);

    match cli.command {
        Command::GetVersion {
            fallback,
            package,
            include_prereleases,
            tag,
        } => {
            let cocogitto = CocoGitto::get()?;
            cocogitto.get_latest_version(fallback, package, include_prereleases, tag)?
        }
        Command::Bump {
            version,
            auto,
            major,
            minor,
            patch,
            pre,
            build,
            hook_profile,
            package,
            annotated,
            dry_run,
            skip_ci,
            skip_ci_override,
            skip_untracked,
            disable_bump_commit,
            include_packages,
        } => {
            let mut cocogitto = CocoGitto::get()?;
            let is_monorepo = !SETTINGS.packages.is_empty();

            let increment = match version {
                Some(version) => IncrementCommand::Manual(version),
                None if auto => match package.as_ref() {
                    Some(package) => {
                        if !is_monorepo {
                            bail!("Cannot create package version on non mono-repository config")
                        };

                        IncrementCommand::AutoPackage(package.to_owned())
                    }
                    None => IncrementCommand::Auto,
                },
                None if major => IncrementCommand::Major,
                None if minor => IncrementCommand::Minor,
                None if patch => IncrementCommand::Patch,
                _ => unreachable!(),
            };

            if is_monorepo {
                match package {
                    Some(package_name) => {
                        // Safe unwrap here, package name is validated by clap
                        let package = SETTINGS.packages.get(&package_name).unwrap();

                        let opts = PackageBumpOptions {
                            package_name: &package_name,
                            package,
                            increment,
                            pre_release: pre.as_deref(),
                            build: build.as_deref(),
                            hooks_config: hook_profile.as_deref(),
                            annotated,
                            dry_run,
                            skip_ci,
                            skip_ci_override,
                            skip_untracked,
                            disable_bump_commit,
                        };

                        cocogitto.create_package_version(opts)?
                    }
                    None => {
                        let opts = BumpOptions {
                            increment,
                            pre_release: pre.as_deref(),
                            build: build.as_deref(),
                            hooks_config: hook_profile.as_deref(),
                            annotated,
                            dry_run,
                            skip_ci,
                            skip_ci_override,
                            skip_untracked,
                            disable_bump_commit,
                            include_packages,
                        };

                        cocogitto.create_monorepo_version(opts)?
                    }
                }
            } else {
                let opts = BumpOptions {
                    increment,
                    pre_release: pre.as_deref(),
                    build: build.as_deref(),
                    hooks_config: hook_profile.as_deref(),
                    annotated,
                    dry_run,
                    skip_ci,
                    skip_ci_override,
                    skip_untracked,
                    disable_bump_commit,
                    include_packages,
                };
                cocogitto.create_version(opts)?
            }
        }
        Command::Verify {
            message,
            file,
            ignore_merge_commits,
            ignore_fixup_commits,
        } => {
            let ignore_merge_commits = ignore_merge_commits || SETTINGS.ignore_merge_commits;
            let ignore_fixup_commits = ignore_fixup_commits || SETTINGS.ignore_fixup_commits;
            let author = CocoGitto::get()
                .map(|cogito| cogito.get_committer().unwrap())
                .ok();

            let commit_message = match (message, file) {
                (Some(message), None) => message,
                (None, Some(file_path)) => {
                    if !file_path.exists() {
                        bail!("File {file_path:#?} does not exist");
                    }

                    match fs::read_to_string(file_path) {
                        Err(e) => bail!("Could not read the file ({e})"),
                        Ok(msg) => msg,
                    }
                }
                (None, None) => unreachable!(),
                (Some(_), Some(_)) => unreachable!(),
            };

            conv_commit::verify(
                author,
                &commit_message,
                ignore_merge_commits,
                ignore_fixup_commits,
            )?;
        }
        Command::Check {
            from_latest_tag,
            ignore_merge_commits,
            ignore_fixup_commits,
            range,
        } => {
            let cocogitto = CocoGitto::get()?;
            let from_latest_tag = from_latest_tag || SETTINGS.from_latest_tag;
            let ignore_merge_commits = ignore_merge_commits || SETTINGS.ignore_merge_commits;
            let ignore_fixup_commits = ignore_fixup_commits || SETTINGS.ignore_fixup_commits;
            cocogitto.check(
                from_latest_tag,
                ignore_merge_commits,
                ignore_fixup_commits,
                range,
            )?;
        }
        Command::Edit { from_latest_tag } => {
            let cocogitto = CocoGitto::get()?;
            let from_latest_tag = from_latest_tag || SETTINGS.from_latest_tag;
            cocogitto.check_and_edit(from_latest_tag)?;
        }
        Command::Log {
            breaking_change,
            typ,
            author,
            scope,
            no_error,
        } => {
            let cocogitto = CocoGitto::get()?;

            let repo_tag_name = cocogitto.get_repo_tag_name();
            let repo_tag_name = repo_tag_name.as_deref().unwrap_or("cog log");

            let mut output = Output::builder()
                .with_pager_from_env("PAGER")
                .with_file_name(repo_tag_name)
                .build()?;

            let mut filters = vec![];
            if let Some(commit_types) = typ {
                filters.extend(
                    commit_types
                        .iter()
                        .map(|commit_type| CommitFilter::Type(commit_type.as_str().into())),
                );
            }

            if let Some(scopes) = scope {
                filters.extend(scopes.into_iter().map(CommitFilter::Scope));
            }

            if let Some(authors) = author {
                filters.extend(authors.into_iter().map(CommitFilter::Author));
            }

            if breaking_change {
                filters.push(CommitFilter::BreakingChange);
            }

            if no_error {
                filters.push(CommitFilter::NoError);
            }

            let filters = CommitFilters(filters);

            let content = cocogitto.get_log(filters)?;
            output
                .handle()?
                .write_all(content.as_bytes())
                .context("failed to write log into the pager")?;
        }
        Command::Changelog {
            pattern,
            at,
            template,
            remote,
            owner,
            repository,
            unified,
        } => {
            let cocogitto = CocoGitto::get()?;

            let context = RemoteContext::try_new(remote, repository, owner)
                .or_else(|| SETTINGS.get_template_context());
            let template = template.as_ref().or(SETTINGS.changelog.template.as_ref());
            let template = if let Some(template) = template {
                Template::from_arg(template, context)?
            } else {
                // fallback never uses remote context
                Template::fallback(unified)
            };

            let pattern = at.or(pattern);
            let pattern = pattern.as_deref().unwrap_or("..");
            let result = if !SETTINGS.packages.is_empty() {
                cocogitto.get_monorepo_changelog(pattern, template, unified)?
            } else {
                let changelog = cocogitto.get_changelog(pattern, true)?;
                changelog.into_markdown(template, ReleaseType::Standard)?
            };
            println!("{result}");
        }
        Command::Init { path } => {
            cocogitto::command::init::init(&path)?;
        }
        Command::InstallHook {
            hook_type: hook_types,
            all,
            overwrite,
        } => {
            let cocogitto = CocoGitto::get()?;
            if all {
                cocogitto.install_all_hooks(overwrite)?;
                return Ok(());
            };

            let hook_types = hook_types.into_iter().map(GitHookType::from).collect();

            cocogitto.install_git_hooks(overwrite, hook_types)?;
        }
        Command::GenerateCompletions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "cog", &mut std::io::stdout());
        }
        Command::GenerateManpages { output_dir } => {
            mangen::generate_manpages(&output_dir)?;
        }
        Command::Commit(CommitArgs {
            typ,
            message,
            scope,
            breaking_change,
            edit,
            sign,
            skip_ci,
            skip_ci_override,
            add_files,
            update_files,
        }) => {
            let cocogitto = CocoGitto::get()?;
            cocogitto.run_commit_hook(CommitHook::PreCommit)?;
            let commit_message_path = cocogitto.prepare_edit_message_path();

            let commit_message = if skip_ci || skip_ci_override.is_some() {
                format!(
                    "{} {}",
                    message,
                    skip_ci_override.unwrap_or(SETTINGS.skip_ci.clone())
                )
            } else {
                message
            };

            let template = prepare_edit_message(
                &typ,
                &commit_message,
                scope.as_deref(),
                breaking_change,
                &commit_message_path,
            )?;
            cocogitto.run_commit_hook(CommitHook::PrepareCommitMessage(template))?;

            let (body, footer, breaking) = if edit {
                commit::edit_message(&commit_message_path, breaking_change)?
            } else {
                (None, None, breaking_change)
            };

            let opts = CommitOptions {
                commit_type: &typ,
                scope,
                summary: commit_message,
                body,
                footer,
                breaking,
                sign,
                add_files,
                update_files,
            };

            cocogitto.conventional_commit(opts)?;
            cocogitto.run_commit_hook(CommitHook::PostCommit)?;
        }
        Command::Target(targets) => {
            let targets = targets.get_targets()?;
            if targets.standard {
                println!("selected standard");
            }
            if targets.global {
                println!("selected global");
            }
            for pkg in &targets.packages {
                println!("selected package {pkg}");
            }
        }
    }

    println!();
    Ok(())
}

fn init_logs(verbose: u8, quiet: bool) {
    let verbosity = if verbose == 0 { 2 } else { verbose - 1 };
    stderrlog::new()
        .module(module_path!())
        .modules(vec!["cocogitto"])
        .quiet(quiet)
        .verbosity(verbosity as usize)
        .show_level(false)
        .show_module_names(false)
        .init()
        .unwrap();
}
