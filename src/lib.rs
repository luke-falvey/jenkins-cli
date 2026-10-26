//! Read-only command line client for the Jenkins REST API on CloudBees CI.

pub mod cli;
pub mod client;
pub mod credentials;
pub mod models;
pub mod output;

use std::io::Write;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::Value;
use thiserror::Error;

use crate::cli::{
    BuildsCommand, Cli, Command, JobsCommand, NodesCommand, OutputFormat, PluginsCommand,
    QueueCommand, ViewsCommand,
};
use crate::client::{ClientError, JenkinsClient};
use crate::credentials::{Credentials, CredentialsError};
use crate::models::{
    Build, Computer, ComputerSet, Controller, Job, PluginManager, Queue, QueueItem, View, WhoAmI,
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("no Jenkins URL given; pass --url or set JENKINS_URL")]
    MissingUrl,
    #[error(transparent)]
    Credentials(#[from] CredentialsError),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("the response did not have the expected shape")]
    Shape(#[source] serde_json::Error),
    #[error("could not write output")]
    Output(#[from] std::io::Error),
}

/// Builds a client from the global arguments, reading the credentials file.
///
/// Returns any warning to show the user (for example about file permissions).
pub fn client_from_args(cli: &Cli) -> Result<(JenkinsClient, Option<String>), Error> {
    let url = cli.global.url.as_deref().ok_or(Error::MissingUrl)?;
    let path = match &cli.global.credentials_file {
        Some(path) => path.clone(),
        None => Credentials::default_path()?,
    };
    let credentials = Credentials::from_file(&path)?;
    let warning = credentials::permissions_warning(&path);
    let client = JenkinsClient::new(url, credentials, Duration::from_secs(cli.global.timeout))?;
    Ok((client, warning))
}

fn parse<T: DeserializeOwned>(value: Value) -> Result<T, Error> {
    serde_json::from_value(value).map_err(Error::Shape)
}

/// Prints `value` as JSON, or converts it to `T` and prints it as text.
fn emit<T: DeserializeOwned>(
    out: &mut impl Write,
    format: OutputFormat,
    value: Value,
    render: impl FnOnce(&T) -> String,
) -> Result<(), Error> {
    match format {
        OutputFormat::Json => {
            serde_json::to_writer_pretty(&mut *out, &value).map_err(Error::Shape)?;
            writeln!(out)?;
        }
        OutputFormat::Text => out.write_all(render(&parse(value)?).as_bytes())?,
    }
    Ok(())
}

/// Runs a command and writes its output to `out`.
pub async fn run(
    client: &JenkinsClient,
    command: &Command,
    format: OutputFormat,
    out: &mut impl Write,
) -> Result<(), Error> {
    match command {
        Command::Whoami => {
            emit::<WhoAmI>(out, format, client.who_am_i().await?, output::who_am_i)?;
        }
        Command::Info => {
            emit::<Controller>(
                out,
                format,
                client.controller(None).await?,
                output::controller,
            )?;
        }
        Command::Jobs(JobsCommand::List { folder }) => {
            let value = match folder {
                Some(folder) => client.job(folder, Some(client::JOBS_TREE)).await?,
                None => client.controller(Some(client::JOBS_TREE)).await?,
            };
            emit::<Job>(out, format, value, |j| output::items(&j.jobs))?;
        }
        Command::Jobs(JobsCommand::Get { job }) => {
            emit::<Job>(out, format, client.job(job, None).await?, output::job)?;
        }
        Command::Builds(BuildsCommand::List { job, limit }) => {
            let tree = client::builds_tree(*limit);
            let value = client.job(job, Some(&tree)).await?;
            emit::<Job>(out, format, value, |j| output::builds(&j.builds))?;
        }
        Command::Builds(BuildsCommand::Get(args)) => {
            let value = client.build(&args.job, &args.build).await?;
            emit::<Build>(out, format, value, output::build)?;
        }
        Command::Builds(BuildsCommand::Log(args)) => {
            // The log is plain text whatever the output format.
            let log = client.console_text(&args.job, &args.build).await?;
            out.write_all(log.as_bytes())?;
            if !log.is_empty() && !log.ends_with('\n') {
                writeln!(out)?;
            }
        }
        Command::Queue(QueueCommand::List) => {
            emit::<Queue>(out, format, client.queue().await?, output::queue)?;
        }
        Command::Queue(QueueCommand::Get { id }) => {
            emit::<QueueItem>(
                out,
                format,
                client.queue_item(*id).await?,
                output::queue_item,
            )?;
        }
        Command::Nodes(NodesCommand::List) => {
            emit::<ComputerSet>(out, format, client.computers().await?, output::computers)?;
        }
        Command::Nodes(NodesCommand::Get { name }) => {
            emit::<Computer>(out, format, client.computer(name).await?, output::computer)?;
        }
        Command::Views(ViewsCommand::List) => {
            let value = client.controller(Some(client::VIEWS_TREE)).await?;
            emit::<Controller>(out, format, value, |c| output::views(&c.views))?;
        }
        Command::Views(ViewsCommand::Get { name }) => {
            emit::<View>(out, format, client.view(name).await?, output::view)?;
        }
        Command::Plugins(PluginsCommand::List { updates }) => {
            emit::<PluginManager>(out, format, client.plugins().await?, |p| {
                output::plugins(p, *updates)
            })?;
        }
    }
    out.flush()?;
    Ok(())
}
