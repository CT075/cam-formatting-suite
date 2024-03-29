use std::path::PathBuf;

use anyhow::Result;
use clap::{ArgAction, Parser, Subcommand};

mod driver;
mod parsefile;
mod textprocess;

// CR-someday cam: colored help output?
/// Text formatting suite for use with Event Assembler
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    mode: Mode,
    /// Definitions file
    #[arg(short, long)]
    defs: Option<PathBuf>,
    /// Allow arbitrary unicode characters.
    #[arg(long, action=ArgAction::SetTrue, global=true)]
    utf8: bool,
    /// Automatically insert [NL] in place of newlines.
    #[arg(long, action=ArgAction::SetTrue, global=true)]
    auto_newline: bool,
    /// Automatically insert [X] at the end of each text entry, if there isn't
    /// already one
    #[arg(long, action=ArgAction::SetTrue, global=true)]
    auto_insert_x: bool,
    /// Automatically remove leading whitespace before processing a text entry
    #[arg(long, action=ArgAction::SetTrue, global=true)]
    strip_leading_whitespace: bool,
    /// Automatically remove trailing whitespace before processing a text
    /// entry. This will not strip any whitespace before a manual [X].
    #[arg(long, action=ArgAction::SetTrue, global=true)]
    strip_trailing_whitespace: bool,
    /// Activate both --strip-leading-whitespace and --strip-trailing-whitespace
    #[arg(long, action=ArgAction::SetTrue, global=true)]
    strip_whitespace: bool,
}

#[derive(Subcommand, Debug)]
enum Mode {
    /// Process a single file
    #[clap(alias("parsefile"))]
    Single {
        /// Output DMP
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Write to stdout (at least one of --output and --to-stdout must be set)
        #[arg(long, action=ArgAction::SetFalse)]
        to_stdout: bool,
        #[arg()]
        input: PathBuf,
    },
    #[clap(alias("textprocess"))]
    Batch,
}

fn main() -> Result<()> {
    let args = Args::parse();

    match args.mode {
        Mode::Single {
            output,
            to_stdout,
            input,
        } => {
            let (strip_leading_whitespace, strip_trailing_whitespace) =
                if args.strip_whitespace {
                    (true, true)
                } else {
                    (
                        args.strip_leading_whitespace,
                        args.strip_trailing_whitespace,
                    )
                };

            let cfg = parsefile::Config {
                auto_newline: args.auto_newline,
                auto_insert_x: args.auto_insert_x,
                strip_leading_whitespace,
                strip_trailing_whitespace,
                utf8: args.utf8,
            };

            driver::parse_single(output, to_stdout, input, args.defs, cfg)
        }
        Mode::Batch => todo!(),
    }
}
