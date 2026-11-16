//! The `slidock` command: `slidock compile <input>` (or `slidock <input>`) writes the input's deck, `<input
//! stem>.zip` in the working folder unless `-o` names a zip or a folder; `--emit ir|documents|report` writes those
//! instead. `slidock licenses` prints the licences of slidock and of what it is built from. Exit codes: 0 success, 1
//! the compile reported errors or the output could not be written, 2 a refused input or bad usage.

mod bundle;
mod bundled;
mod clock;
mod compile;
mod files;
mod fonts;
mod licenses;
mod say;
mod sources;

use std::ffi::OsString;
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};

const EXIT_CODES: &str = "\
Exit status:
  0  The deck (or --emit output) was written. Warnings and losses do not change it.
  1  The compile reported errors: a slide, layout or master that could not be converted is left out of the deck,
     and a presentation that fails as a whole, or an output that cannot be written, leaves no deck.
  2  The input was refused (not a presentation, password-protected, over a limit) or the command line is invalid.";

#[derive(Debug, Parser)]
#[command(
    name = "slidock",
    version,
    about = "Compiles PowerPoint, Keynote and OpenDocument presentations to standalone HTML decks",
    arg_required_else_help = true,
    override_usage = "slidock [compile] <INPUT> [OPTIONS]\n       slidock <COMMAND>",
    after_help = format!(
        "Examples:\n  \
         slidock talk.pptx             writes talk.zip in the working folder\n  \
         slidock talk.key -o site/     writes the deck into the folder site/\n  \
         slidock talk.odp --offline    compiles without any network request\n\n\
         By default, fonts a presentation neither embeds nor finds elsewhere are downloaded from Google Fonts;\n\
         --offline makes no network request. `slidock compile --help` lists every option.\n\n{EXIT_CODES}"
    )
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compiles one presentation into a deck.
    #[command(after_help = EXIT_CODES)]
    Compile(Box<compile::CompileArgs>),
    /// Prints the licences of slidock and of everything it is built from.
    Licenses,
}

/// How the command ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    Ok,
    /// The compile reported errors, or an output could not be written.
    Failed,
    /// A refused input or bad usage.
    Usage,
}

impl From<Exit> for ExitCode {
    fn from(exit: Exit) -> Self {
        ExitCode::from(match exit {
            Exit::Ok => 0,
            Exit::Failed => 1,
            Exit::Usage => 2,
        })
    }
}

/// The arguments with `compile` put first when the first one is neither an option nor a subcommand, so `slidock
/// talk.pptx` runs `slidock compile talk.pptx`.
fn with_default_command(mut args: Vec<OsString>) -> Vec<OsString> {
    let Some(first) = args.get(1) else {
        return args;
    };
    // Only a path can be other than UTF-8.
    let known = first.to_str().is_some_and(|first| {
        first.starts_with('-')
            || first == "help"
            || Cli::command().get_subcommands().any(|sub| {
                sub.get_name() == first || sub.get_all_aliases().any(|alias| alias == first)
            })
    });
    if !known {
        args.insert(1, "compile".into());
    }
    args
}

fn main() -> ExitCode {
    let cli = Cli::parse_from(with_default_command(std::env::args_os().collect()));
    match cli.command {
        Command::Compile(args) => compile::run(*args).into(),
        Command::Licenses => licenses::run().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        with_default_command(list.iter().map(OsString::from).collect())
            .into_iter()
            .map(|arg| arg.into_string().unwrap())
            .collect()
    }

    #[test]
    fn a_first_argument_that_is_no_command_or_option_is_compiled() {
        assert_eq!(
            args(&["slidock", "talk.pptx", "-o", "t.zip"]),
            ["slidock", "compile", "talk.pptx", "-o", "t.zip"]
        );
        for kept in [
            &["slidock"][..],
            &["slidock", "compile", "talk.pptx"],
            &["slidock", "--help"],
            &["slidock", "-V"],
            &["slidock", "help", "compile"],
        ] {
            assert_eq!(args(kept), kept, "{kept:?}");
        }
    }

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
    }
}
