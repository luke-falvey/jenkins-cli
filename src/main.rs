use std::error::Error as _;
use std::io::{self, ErrorKind};
use std::process::ExitCode;

use clap::Parser;

use jenkins_cli::cli::Cli;
use jenkins_cli::{Error, client_from_args, run};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match try_main(&cli).await {
        Ok(()) => ExitCode::SUCCESS,
        // Output piped into something like `head` that stopped reading.
        Err(Error::Output(e)) if e.kind() == ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(err) => {
            let mut message = format!("error: {err}");
            let mut source = err.source();
            while let Some(cause) = source {
                message.push_str(&format!("\n  caused by: {cause}"));
                source = cause.source();
            }
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

async fn try_main(cli: &Cli) -> Result<(), Error> {
    let (client, warning) = client_from_args(cli)?;
    if let Some(warning) = warning {
        eprintln!("{warning}");
    }
    let stdout = io::stdout();
    let mut out = stdout.lock();
    run(&client, &cli.command, cli.global.output, &mut out).await
}
