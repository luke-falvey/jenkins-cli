//! Command line definition.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::models::BuildSelector;

/// Read-only command line client for Jenkins on CloudBees CI.
///
/// Credentials are read from ~/.cloudbees_credentials, a file holding a single
/// line `username:password`. Use an API token as the password.
#[derive(Debug, Parser)]
#[command(name = "jenkins-cli", version, about, long_about)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Args)]
pub struct GlobalArgs {
    /// Base URL of the CloudBees CI controller or Jenkins instance,
    /// for example https://cloudbees.example.com/my-controller/
    #[arg(long, env = "JENKINS_URL", global = true, value_name = "URL")]
    pub url: Option<String>,

    /// Credentials file to read instead of ~/.cloudbees_credentials
    #[arg(
        long,
        env = "CLOUDBEES_CREDENTIALS_FILE",
        global = true,
        value_name = "PATH"
    )]
    pub credentials_file: Option<PathBuf>,

    /// Output format
    #[arg(short, long, value_enum, default_value_t = OutputFormat::Text, global = true)]
    pub output: OutputFormat,

    /// Request timeout in seconds
    #[arg(long, default_value_t = 30, global = true, value_name = "SECONDS")]
    pub timeout: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    /// Readable tables and summaries
    Text,
    /// The JSON returned by Jenkins, pretty-printed
    Json,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show the user you are authenticated as
    Whoami,
    /// Show an overview of the controller
    Info,
    /// List and inspect jobs, folders and multibranch projects
    #[command(subcommand)]
    Jobs(JobsCommand),
    /// List and inspect builds of a job
    #[command(subcommand)]
    Builds(BuildsCommand),
    /// Inspect the build queue
    #[command(subcommand)]
    Queue(QueueCommand),
    /// List and inspect nodes (agents)
    #[command(subcommand)]
    Nodes(NodesCommand),
    /// List and inspect views
    #[command(subcommand)]
    Views(ViewsCommand),
    /// List installed plugins
    #[command(subcommand)]
    Plugins(PluginsCommand),
}

#[derive(Debug, Subcommand)]
pub enum JobsCommand {
    /// List the jobs at the top level or inside a folder
    List {
        /// Folder to list, for example `team-a` or `team-a/services`
        #[arg(long, value_name = "FOLDER")]
        folder: Option<String>,
    },
    /// Show one job
    Get {
        /// Full job name, with folders separated by `/`, for example `team-a/api-service`
        job: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum BuildsCommand {
    /// List the most recent builds of a job
    List {
        /// Full job name, with folders separated by `/`
        job: String,
        /// Number of builds to show
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: u32,
    },
    /// Show one build
    Get(BuildArgs),
    /// Print the console log of a build
    Log(BuildArgs),
}

#[derive(Debug, Args)]
pub struct BuildArgs {
    /// Full job name, with folders separated by `/`
    pub job: String,
    /// Build number, or `last`, `lastSuccessfulBuild`, `lastFailedBuild`, ...
    #[arg(default_value = "lastBuild")]
    pub build: BuildSelector,
}

#[derive(Debug, Subcommand)]
pub enum QueueCommand {
    /// List the items waiting in the queue
    List,
    /// Show one queue item, including the build it started
    Get {
        /// Queue item ID
        id: u64,
    },
}

#[derive(Debug, Subcommand)]
pub enum NodesCommand {
    /// List all nodes
    List,
    /// Show one node. The built-in node is named `(built-in)`
    Get {
        /// Node name
        name: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ViewsCommand {
    /// List the top-level views
    List,
    /// Show one view and its jobs
    Get {
        /// View name
        name: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum PluginsCommand {
    /// List installed plugins
    List {
        /// Only show plugins with an update available
        #[arg(long)]
        updates: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_build_selector_with_default() {
        let cli = Cli::parse_from(["jenkins-cli", "builds", "get", "deploy"]);
        let Command::Builds(BuildsCommand::Get(args)) = cli.command else {
            panic!("wrong command");
        };
        assert_eq!(args.build, BuildSelector::Permalink("lastBuild"));
    }

    #[test]
    fn rejects_bad_build_selector() {
        let err =
            Cli::try_parse_from(["jenkins-cli", "builds", "get", "deploy", "nope"]).unwrap_err();
        assert!(err.to_string().contains("expected a build number"), "{err}");
    }

    #[test]
    fn global_flags_work_after_subcommand() {
        let cli = Cli::parse_from([
            "jenkins-cli",
            "jobs",
            "list",
            "--url",
            "https://cb.example.com/",
            "-o",
            "json",
        ]);
        assert_eq!(cli.global.url.as_deref(), Some("https://cb.example.com/"));
        assert_eq!(cli.global.output, OutputFormat::Json);
    }
}
