use std::collections::HashMap;

use anyhow::Result;

use crate::parsefile;

pub struct Config {
    allow_c_line_comments: bool,
}

pub fn chomp_leading_definitions(
    lines: impl Iterator<Item = Result<String>>,
    parsefile_config: &parsefile::Config,
) -> Result<(HashMap<String, Vec<u8>>, Option<String>)> {
    let mut result = HashMap::new();

    for line in lines {
        let line = line?;
        if line.trim().starts_with("#") {
            return Ok((result, Some(line)));
        }
        if let Some((name, body)) =
            parsefile::parse_definition(line, &result, parsefile_config.utf8)?
        {
            result.insert(name, body);
        }
    }

    return Ok((result, None));
}

pub fn chomp_next_entry(
    lines: impl Iterator<Item = Result<String>>,
    cfg: &Config,
) -> Result<(String, Option<String>)> {
    let mut result = String::new();

    for line in lines {
        let line = line?;
        if line.starts_with("#") {
            return Ok((result, Some(line)));
        }

        if cfg.allow_c_line_comments {
            // to remove C line comments, take the part of the string before any "//"
            for t in line.split("//") {
                result.push_str(&t);
                break;
            }
        } else {
            result.push_str(&line);
        }
        result.push('\n');
    }

    return Ok((result, None));
}
