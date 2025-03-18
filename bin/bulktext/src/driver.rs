use std::{
    collections::HashMap,
    fs::{self, File},
    io::{prelude::*, BufReader, BufWriter, Lines},
    path::Path,
};

use anyhow::{anyhow, bail, Result};

use crate::{parsefile, textprocess};

pub struct Config {
    pub parsefile: parsefile::Config,
    pub textprocess: textprocess::Config,
}

pub fn parse_single(
    output: Option<impl AsRef<Path>>,
    to_stdout: bool,
    input: impl AsRef<Path>,
    defs: Option<impl AsRef<Path>>,
    cfg: parsefile::Config,
) -> Result<()> {
    if !matches!(output, Some(_)) && !to_stdout {
        bail!("at least one of --output or --to-stdout must be set")
    }

    let definitions = if let Some(defs) = defs {
        parse_definitions_file(&defs, cfg.utf8).map_err(|e| {
            e.context(format!(
                "while processing definitions file `{0}`",
                defs.as_ref().display()
            ))
        })?
    } else {
        HashMap::new()
    };

    let result = parse_one_file(input, &definitions, &cfg)?;

    if let Some(output) = output {
        fs::write(output, result)?;
    }

    if to_stdout {}

    Ok(())
}

fn parse_definitions_file(
    path: impl AsRef<Path>,
    utf8: bool,
) -> Result<HashMap<String, Vec<u8>>> {
    let handle = File::open(path)?;
    let reader = BufReader::new(handle);

    let mut definitions = HashMap::new();

    for line in reader.lines() {
        match parsefile::parse_definition(line?, &definitions, utf8)? {
            None => continue,
            Some((name, value)) => {
                definitions.insert(name, value);
            }
        }
    }

    return Ok(definitions);
}

fn parse_one_file(
    path: impl AsRef<Path>,
    definitions: &HashMap<String, Vec<u8>>,
    config: &parsefile::Config,
) -> Result<Vec<u8>> {
    let text = fs::read_to_string(path)?;

    parsefile::parse_text(&text, definitions, config)
}

struct PrintSink {
    stdout: bool,
    file: Option<BufWriter<File>>,
}

impl PrintSink {
    fn stdout() -> Self {
        Self {
            stdout: true,
            file: None,
        }
    }

    fn with_file(p: impl AsRef<Path>) -> Result<Self> {
        let handle = File::open(p)?;
        let writer = BufWriter::new(handle);

        Ok(Self {
            stdout: false,
            file: Some(writer),
        })
    }

    fn new(stdout: bool, p: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            stdout,
            ..Self::with_file(p)?
        })
    }

    fn println(&mut self, s: impl AsRef<String>) -> Result<()> {
        if self.stdout {
            println!("{}", s.as_ref());
        }
        match self.file {
            None => {}
            Some(ref mut file) => {
                file.write_all(s.as_ref().as_bytes())?;
                file.write(&[b'\n'])?;
            }
        }

        Ok(())
    }
}

struct FileStack {
    stack: Vec<(String, Lines<BufReader<File>>)>,
}

impl FileStack {
    fn new(path: impl AsRef<Path>) -> Result<Self> {
        let mut result = Self { stack: Vec::new() };

        result.push_file(path);

        Ok(result)
    }

    fn push_file(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let lines = BufReader::new(File::open(&path)?).lines();

        self.stack
            .push((format!("{}", path.as_ref().display()), lines));

        Ok(())
    }
}

impl Iterator for FileStack {
    type Item = Result<String>;

    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        let (top_name, top) = self.stack.last_mut()?;
        if let Some(s) = top.next() {
            return Some(s.map_err(|e| {
                anyhow!(e).context(format!("couldn't read file {0}", top_name))
            }));
        }

        self.stack.pop();
        self.next()
    }
}

fn process_alltext(
    path: impl AsRef<Path>,
    prior_definitions: &HashMap<String, Vec<u8>>,
    config: &Config,
    sink: PrintSink,
) -> Result<String> {
    let mut lines = FileStack::new(&path)?;

    let (mut definitions, mut maybe_header) =
        textprocess::chomp_leading_definitions(
            lines.by_ref(),
            &config.parsefile,
        )?;

    for (k, v) in prior_definitions {
        if definitions.contains_key(k) {
            bail!(
                "[{0}] defined in both definitions file and `{1}`",
                k,
                path.as_ref().display()
            );
        }
        definitions.insert(k.clone(), v.clone());
    }

    let definitions = definitions;

    loop {
        let header = if let Some(header) = maybe_header {
            header
        } else {
            break;
        };

        let (block, next_header) =
            textprocess::chomp_next_entry(lines.by_ref(), &config.textprocess)?;

        maybe_header = next_header;
    }

    todo!()
}
