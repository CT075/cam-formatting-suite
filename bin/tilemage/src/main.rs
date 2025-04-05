// XXX: This is the world's most overengineered argument parser.

use std::{env, fs, io::Write, path::PathBuf};

use anyhow::{bail, Result};
use atty;
use clap::{
    error::{ContextKind, ContextValue, DefaultFormatter, Error, ErrorKind},
    ArgAction, Command, CommandFactory, Parser, Subcommand,
};
use image::{ImageFormat, ImageReader};

use gbalz77 as lz77;
use tilemage as gbagfx;

#[derive(Subcommand, Debug)]
enum Mode {
    /// Direct conversion to GBA format.
    Convert(ConvertArgs),
}

#[derive(Parser, Debug)]
#[command(disable_help_flag = true)]
struct ConvertArgs {
    input: PathBuf,
    #[arg(short, long)]
    output: Option<PathBuf>,
    #[arg(short = 'p', long)]
    palette_out: Option<PathBuf>,
    /// Use the specified palette instead of the input image's.
    #[arg(long)]
    palette_in: Option<String>,
    /// Write to stdout. Mutually exclusive with other output options.
    #[arg(long, action=ArgAction::SetTrue)]
    to_stdout: bool,
    /// Output palette only.
    #[arg(long, action=ArgAction::SetTrue)]
    palette_only: bool,
    /// Compress result
    #[arg(long, action=ArgAction::SetTrue)]
    lz77: bool,
    /// Print help information
    #[arg(long, global=true, action=clap::ArgAction::HelpLong)]
    help: Option<bool>,
}

enum Output {
    Stdout,
    File(PathBuf),
}

// validated png2dmp args
struct ConvertOpts {
    input: PathBuf,
    palette: Option<String>,
    output: Option<Output>,
    palette_out: Option<Output>,
    force_stdout: bool,
    lz77: bool,
}

impl ConvertArgs {
    fn validate(self) -> Result<ConvertOpts> {
        use Output::*;

        let mut cmd = ConvertArgs::command();
        let force_stdout = self.to_stdout;

        let (output, palette_out) = if self.palette_only {
            let palette_out = match (self.output, self.palette_out) {
                (None, None) => Some(Stdout),
                (Some(p), None) | (None, Some(p)) => {
                    if force_stdout {
                        cmd.error(ErrorKind::ValueValidation,
                            "--output/--palette-out and --to-stdout are mutually exclusive"
                            ).exit()
                    }
                    Some(File(p))
                }
                (Some(_), Some(_)) => {
                    cmd.error(ErrorKind::ValueValidation,
                        "--palette-only can only be used with at most one of --output or --palette-out").exit()
                }
            };
            (None, palette_out)
        } else {
            let output = match (self.output, force_stdout) {
                (Some(_), true) =>
                        cmd.error(ErrorKind::ValueValidation,
                            "--output/--palette-out and --to-stdout are mutually exclusive"
                            ).exit(),
                (Some(fname), false) => Some(File(fname)),
                (None, _) => Some(Stdout),
            };
            let palette_out = self.palette_out.map(File);

            (output, palette_out)
        };

        Ok(ConvertOpts {
            input: self.input,
            palette: self.palette_in,
            output,
            palette_out,
            force_stdout,
            lz77: self.lz77,
        })
    }
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None, disable_help_flag = true)]
struct Args {
    #[command(subcommand)]
    mode: Mode,
}

fn check_arg_path<T>(
    cmd: &Command,
    ctx: &str,
    arg: Option<&String>,
    f: impl Fn(String) -> T,
) -> Result<T> {
    match arg {
        Some(arg) if !arg.starts_with("-") => Ok(f(arg.clone())),
        _ => {
            let mut err: Error<DefaultFormatter> =
                Error::new(ErrorKind::InvalidValue).with_cmd(cmd);
            err.insert(
                ContextKind::InvalidArg,
                ContextValue::String(ctx.to_string()),
            );
            err.insert(
                ContextKind::InvalidValue,
                ContextValue::String("".to_string()),
            );
            err.exit();
        }
    }
}

// CR-someday cam: We might be able to generate this function automatically
fn legacy_argparse(args: &[String]) -> Result<Args> {
    let mut output = None;
    let mut palette_out = None;
    let mut palette_in = None;
    let mut to_stdout = false;
    let mut palette_only = false;
    let mut lz77 = false;
    let mut help = None;
    let mut input = None;

    let cmd = ConvertArgs::command();

    let mut args = args.iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--lz77" => lz77 = true,
            "-up" => {
                palette_in = Some(check_arg_path(
                    &cmd,
                    "--palette-in <PALETTE>",
                    args.next(),
                    String::from,
                )?)
            }
            "-po" => {
                palette_out = Some(check_arg_path(
                    &cmd,
                    "--palette-out <OUTPUT>",
                    args.next(),
                    PathBuf::from,
                )?)
            }
            "-o" => {
                output = Some(check_arg_path(
                    &cmd,
                    "--output <OUTPUT>",
                    args.next(),
                    PathBuf::from,
                )?)
            }
            "--palette-only" => palette_only = true,
            "--to-stdout" => to_stdout = true,
            "--help" => help = Some(true),
            _ => match input {
                None => input = Some(PathBuf::from(arg)),
                Some(_) => {
                    let mut err: Error<DefaultFormatter> =
                        Error::new(ErrorKind::UnknownArgument).with_cmd(&cmd);
                    err.insert(
                        ContextKind::InvalidArg,
                        ContextValue::String(arg.to_string()),
                    );
                    err.exit()
                }
            },
        }
    }

    if let Some(true) = help {
        let mut cmd = Args::command();
        let subcmd = cmd.find_subcommand_mut("convert").unwrap();
        subcmd.print_help()?;
        std::process::exit(0);
    }

    if let Some(input) = input {
        Ok(Args {
            mode: Mode::Convert(ConvertArgs {
                input,
                output,
                palette_out,
                palette_in,
                to_stdout,
                palette_only,
                lz77,
                help,
            }),
        })
    } else {
        let mut err: Error<DefaultFormatter> =
            Error::new(ErrorKind::MissingRequiredArgument).with_cmd(&cmd);
        err.insert(
            ContextKind::InvalidArg,
            ContextValue::Strings(vec!["<INPUT>".to_string()]),
        );
        err.exit();
    }
}

fn load_palette(s: impl AsRef<str>) -> Result<gbagfx::Palette> {
    match gbagfx::parse_palette_string(s.as_ref()) {
        Some(p) => return Ok(p),
        None => (),
    }

    if vec![".dmp", ".bin", ".pal"]
        .iter()
        .any(|suffix| s.as_ref().to_lowercase().ends_with(suffix))
    {
        let data = fs::read(s.as_ref())?;
        Ok(data.into_iter().collect())
    } else {
        let image = ImageReader::open(s.as_ref())?.decode()?;
        Ok(gbagfx::read_colors_from_image(&image))
    }
}

fn maybe_compress(lz77: bool, data: Vec<u8>) -> Vec<u8> {
    if lz77 {
        lz77::compress(&data[..], lz77::CompressionStrategy::CheckAllCandidates)
    } else {
        data
    }
}

fn write_target(
    target: Output,
    data: Vec<u8>,
    force_stdout: bool,
) -> Result<()> {
    match target {
        Output::Stdout => try_write_stdout(data, force_stdout)?,
        Output::File(path) => fs::write(path, data)?,
    }

    Ok(())
}

fn try_write_stdout(data: Vec<u8>, force: bool) -> Result<()> {
    if atty::is(atty::Stream::Stdout) && !force {
        eprintln!(
            "Warning: It looks like you're running tilemage in a terminal."
        );
        eprintln!("Warning: Printing raw binary output to your terminal can cause problems.");
        eprintln!("Warning: If you want to do it anyway, use `--to-stdout`.");
        bail!("aborting")
    }
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let _ = handle.write(&data)?;
    Ok(())
}

impl ConvertOpts {
    fn run(self) -> Result<()> {
        let format = ImageFormat::from_path(&self.input).ok();
        let input = fs::read(self.input)?;

        // We can't write this using `map` because we want to propagate the
        // result from `load_palette` to the outermost `run` function
        let palette = match self.palette {
            Some(s) => Some(load_palette(s)?),
            None => None,
        };

        let image = gbagfx::convert_image(&input[..], format, palette)?;
        image.validate()?;
        let image_was_output = matches!(&self.output, Some(_));

        if let Some(target) = self.output {
            let result: Vec<u8> =
                maybe_compress(self.lz77, gbagfx::encode_tiles(image.tiles()));
            write_target(target, result, self.force_stdout)?;
        }

        if let Some(target) = self.palette_out {
            let result: Vec<u8> = maybe_compress(
                !image_was_output && self.lz77,
                image.palette.encode(),
            );
            write_target(target, result, self.force_stdout)?;
        }

        Ok(())
    }
}

fn main() -> Result<()> {
    let mut args = env::args().into_iter();
    args.next();
    let args = args.collect::<Vec<_>>();

    let args = match args.split_first() {
        Some((cmd, rest)) if cmd.to_lowercase() == "png2dmp" => {
            legacy_argparse(rest)?
        }
        _ => Args::parse(),
    };

    match args.mode {
        Mode::Convert(args) => {
            args.validate()?.run()?;
        }
    }

    Ok(())
}
