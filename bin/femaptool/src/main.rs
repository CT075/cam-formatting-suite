use std::{fs, io::Write, path::PathBuf};

use anyhow::{bail, Result};
use clap::{command, ArgAction, Parser};
use tiled::Loader;

mod femap;
mod tmx;

/// A tool for formatting and inserting GBAFE map data
#[derive(Parser, Debug)]
#[command(version, about, long_about = None, disable_help_flag = true)]
struct Args {
    /// Input file (if no input, read from stdin)
    input: PathBuf,
    /// Output dmp (derived from input if absent)
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// File to write installer to
    #[arg(long)]
    installer_file: Option<PathBuf>,
    /// Label to write map binary data under
    #[arg(long, default_value = "MapData")]
    map_label: String,
    /// Label to write mapchange table entries under
    #[arg(long, default_value = "MapChangeData")]
    mapchanges_label: String,
    /// Print help information
    #[arg(long, global=true, action=clap::ArgAction::HelpLong)]
    help: Option<bool>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // CR-soon cam: Allow people to choose a format directly
    let map = match args.input.extension() {
        Some(ext) => {
            if ext == "tmx" {
                tmx::process_femap(&Loader::new().load_tmx_map(&args.input)?)?
            } else {
                bail!("unrecognized map type")
            }
        }
        _ => bail!("unrecognized map type"),
    };

    let output = match args.output {
        Some(output) => output,
        None => {
            let mut inp = args.input.clone();
            inp.set_extension("dmp");
            inp
        }
    };

    let installer = map
        .installer(args.map_label, args.mapchanges_label, &output)?
        .join("\n");

    fs::write(output, map.encode()?)?;

    if let Some(installer_path) = args.installer_file {
        fs::write(installer_path, installer)?;
    }

    Ok(())
}
