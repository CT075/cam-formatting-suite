use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

use ariadne::{Cache, FileCache, Source};
use clap::Parser;
use toml_edit::ImDocument;

mod entry;
mod nmmtools;
mod schema;
mod toplevel;

struct FC(FileCache);

impl<P> Cache<P> for FC
where
    P: AsRef<Path>,
{
    type Storage = <FileCache as Cache<Path>>::Storage;

    fn fetch(&mut self, path: &P) -> Result<&Source, Box<dyn fmt::Debug + '_>> {
        self.0.fetch(path.as_ref())
    }

    fn display<'a>(&self, path: &'a P) -> Option<Box<dyn fmt::Display + 'a>> {
        self.0.display(path.as_ref())
    }
}

/// A tool for manipulating structs as toml data
#[derive(Parser, Debug)]
#[command(version, about, long_about = None, disable_help_flag = true)]
struct Args {
    /// toml file to process
    #[arg(short, long)]
    input: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let doc = ImDocument::parse(fs::read_to_string(&args.input)?.clone())?;

    match toplevel::handle_document(
        args.input,
        doc,
        None,
        &toplevel::Config {
            skip_unknowns: false,
        },
    ) {
        Ok(lines) => {
            for line in lines {
                println!("{}", line);
            }
        }

        Err(errors) => {
            for err in errors {
                err.eprint(FC(FileCache::default()))?;
            }
        }
    }

    Ok(())
}
