use std::{fs, path::PathBuf};

use anyhow::Result;
use clap::Parser;

use crate::mar::process_mar;

mod mar;

/// Convert Mappy export (.MAR) files to raw binary to be inserted via EA
#[derive(Parser, Debug)]
#[command(version, about, long_about = None, disable_help_flag = true)]
struct Args {
    /// Map width
    #[arg(short, long)]
    width: usize,
    /// Map height
    #[arg(short, long)]
    height: usize,
    /// .MAR file to read
    #[arg(short, long)]
    input: PathBuf,
    /// Output DMP
    #[arg(short, long)]
    output: PathBuf,
    /// Print help information
    #[arg(long, action = clap::ArgAction::HelpLong)]
    help: Option<bool>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let input = fs::read(args.input)?;
    let result = process_mar(args.width, args.height, &input[..])?;
    fs::write(args.output, result)?;

    Ok(())
}
