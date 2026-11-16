//! The fidelity bench: a frozen corpus, oracles fixed before a run, the pinned renderer
//! container driven over CDP, the compiler under test as a child process, `slidock-bench-metrics`' scores and
//! classes, and canonical `results.json` stamped with every input that can move a score.

mod budgets;
mod cache;
mod commands;
mod compare;
mod compile;
mod corpus;
mod determinism;
mod env;
mod error;
mod exec;
mod explain;
mod fonts;
mod fonts_lane;
mod gc;
mod git;
mod golden;
mod gradient;
mod hash;
mod host;
mod images;
mod lo_roundtrip;
mod motion_lane;
mod oracle;
mod parity;
mod renderer;
mod results;
mod runner;
mod server;
mod setup;
mod shadow;
mod stamp;
#[cfg(test)]
mod test_support;

use std::process::ExitCode;

use clap::Parser;

use crate::commands::Cli;
use crate::error::Exit;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("error: {error}");
            return Exit::Failed.into();
        }
    };
    let exit = runtime.block_on(async {
        match commands::run(cli).await {
            Ok(exit) => exit,
            Err(error) => {
                match &error {
                    error::BenchError::Usage(message) => {
                        eprintln!("error: {message}\n\n{}", commands::usage());
                    }
                    _ => eprintln!("{error}"),
                }
                error.exit_code()
            }
        }
    });
    exit.into()
}
