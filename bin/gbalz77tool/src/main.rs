use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
};

use anyhow::{bail, Result};
use atty;
use clap::{ArgAction, ArgGroup, Parser, Subcommand};
use thiserror::Error;

use gbalz77::{
    compress, decompress, BadBlockErrorHandler, CompressionStrategy,
    DecompressErrorHandler,
};

#[derive(Subcommand, Debug)]
enum Mode {
    Compress {
        /// Compress as much as possible (possibly slow, defaults off)
        #[arg(short, action=ArgAction::SetTrue)]
        best: bool,
    },
    Decompress {
        /// Starting offset (inclusive)
        #[arg(short, long = "from")]
        start: Option<usize>,
        /// Ending offset (exclusive)
        #[arg(short, long = "to")]
        end: Option<usize>,
    },
}

/// Utilities for dealing with gbalz77-compressed data.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None, disable_help_flag = true)]
#[clap(group(ArgGroup::new("outkd").args(&["to_stdout", "output"])))]
struct Args {
    #[command(subcommand)]
    mode: Mode,
    /// Input file (if no input, read from stdin)
    #[arg(global = true)]
    input: Option<PathBuf>,
    /// Output file
    #[arg(short, long, global = true, group = "outkd")]
    output: Option<PathBuf>,
    /// Write to stdout (mutually exclusive with [output])
    #[arg(long, global=true, action=ArgAction::SetTrue, group = "outkd")]
    to_stdout: bool,
    /// Print help information
    #[arg(long, global=true, action=clap::ArgAction::HelpLong)]
    help: Option<bool>,
}

#[derive(Error, Debug)]
pub enum DecompressError {
    #[error("Bad reference in block {i:?} (tried to reference index {offs:?}, but data is not long enough)")]
    BadReference { i: usize, offs: usize },
    #[error("input is not long enough to be valid lz77")]
    DataTooShort,
    #[error("invalid header (gbalz77 data must begin with 0x10)")]
    BadHeader,
    #[error("input data is incomplete (got eof, expected {expected:?})")]
    UnexpectedEof { expected: &'static str },
}

impl BadBlockErrorHandler for DecompressError {
    fn bad_reference(i: usize, offs: usize) -> Self {
        Self::BadReference { i, offs }
    }
}

impl DecompressErrorHandler for DecompressError {
    fn data_too_short() -> Self {
        Self::DataTooShort
    }
    fn bad_header() -> Self {
        Self::BadHeader
    }
    fn unexpected_eof(expected: &'static str) -> Self {
        Self::UnexpectedEof { expected }
    }
}

fn main() -> Result<()> {
    let args = Args::parse();

    let input = match args.input {
        None => {
            let mut input = Vec::new();
            let stdin = std::io::stdin();
            let mut handle = stdin.lock();
            handle.read_to_end(&mut input)?;
            input
        }
        Some(fname) => fs::read(fname)?,
    };

    let result = match args.mode {
        Mode::Compress { best } => {
            let strategy = if best {
                CompressionStrategy::CheckAllCandidates
            } else {
                CompressionStrategy::CheckMostRecentOnly
            };
            compress(&input[..], strategy)
        }
        Mode::Decompress { start, end } => {
            let input = match (start, end) {
                (Some(from), Some(to)) => &input[from..to],
                (Some(from), None) => &input[from..],
                (None, Some(to)) => &input[..to],
                (None, None) => &input[..],
            };
            let (result, errs) = decompress::<DecompressError>(input);
            if !errs.is_empty() {
                for err in errs {
                    eprintln!("{err}");
                    bail!("errors encountered during decompression, no output written")
                }
            };
            result
        }
    };

    match (args.output, args.to_stdout) {
        (Some(_), true) => {
            bail!("--output and --to-stdout are mutually exclusive")
        }
        (Some(fname), false) => fs::write(fname, result)?,
        (None, _) => {
            if atty::is(atty::Stream::Stdout) && !args.to_stdout {
                eprintln!("Warning: It looks like you're running gbalz77tool in a terminal.");
                eprintln!("Warning: Printing raw binary output to your terminal can cause problems.");
                eprintln!("Warning: If you want to do it anyway, use `--to-stdout`, or consider `--output`.");
                bail!("aborting")
            }
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            let _ = handle.write(&result)?;
        }
    };

    Ok(())
}
