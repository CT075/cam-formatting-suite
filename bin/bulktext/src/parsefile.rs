use std::{collections::HashMap, sync::OnceLock};

use anyhow::{anyhow, bail, Result};

// CR-someday cam: check game?
fn default_control_codes() -> &'static HashMap<&'static str, &'static [u8]> {
    static HASHMAP: OnceLock<HashMap<&'static str, &'static [u8]>> =
        OnceLock::new();
    HASHMAP.get_or_init(|| {
        let mut m: HashMap<&'static str, &'static [u8]> = HashMap::new();
        m.insert("NL", &[0x01]);
        m.insert("2NL", &[0x02]);
        m.insert("A", &[0x03]);
        m.insert("....", &[0x04]);
        m.insert(".....", &[0x05]);
        m.insert("......", &[0x06]);
        m.insert(".......", &[0x07]);
        m.insert("OpenFarLeft", &[0x08]);
        m.insert("OpenMidLeft", &[0x09]);
        m.insert("OpenLeft", &[0x0A]);
        m.insert("OpenRight", &[0x0B]);
        m.insert("OpenMidRight", &[0x0C]);
        m.insert("OpenFarRight", &[0x0D]);
        m.insert("OpenFarFarLeft", &[0x0E]);
        m.insert("OpenFarFarRight", &[0x0F]);
        m.insert("LoadFace", &[0x10]);
        m.insert("LoadPortrait", &[0x10]); // Alias
        m.insert("ClearFace", &[0x11]);
        m.insert("ClearPortrait", &[0x11]); // Alias again
        m.insert("NormalPrintFE6", &[0x12]); // FE6 Only codes NormalPrint, FastPrint
        m.insert("FastPrintFE6", &[0x13]);
        m.insert("CloseSpeechFast", &[0x14]);
        m.insert("CloseSpeechSlow", &[0x15]);
        m.insert("ToggleMouthMove", &[0x16]);
        m.insert("ToggleSmile", &[0x17]);
        m.insert("Yes", &[0x18]);
        m.insert("No", &[0x19]);
        m.insert("Buy/Sell", &[0x1A]);
        m.insert("ShopContinue", &[0x1B]); // FE8 Only
        m.insert("SendToBack", &[0x1C]);
        m.insert("FastPrint", &[0x1D]);
        m.insert(".", &[0x1F]);
        m.insert("LoadOverworldFaces", &[0x80, 0x04]);
        m.insert("Events", &[0x80, 0x04]); // Alias
        m.insert("G", &[0x80, 0x05]);
        m.insert("MoveFarLeft", &[0x80, 0x0A]);
        m.insert("MoveMidLeft", &[0x80, 0x0B]);
        m.insert("MoveLeft", &[0x80, 0x0C]);
        m.insert("MoveRight", &[0x80, 0x0D]);
        m.insert("MoveMidRight", &[0x80, 0x0E]);
        m.insert("MoveFarRight", &[0x80, 0x0F]);
        m.insert("MoveFarFarLeft", &[0x80, 0x10]);
        m.insert("MoveFarFarRight", &[0x80, 0x11]);
        m.insert("EnableBlinking", &[0x80, 0x16]);
        m.insert("DelayBlinking", &[0x80, 0x18]);
        m.insert("PauseBlinking", &[0x80, 0x19]);
        m.insert("DisableBlinking", &[0x80, 0x1B]);
        m.insert("OpenEyes", &[0x80, 0x1C]);
        m.insert("CloseEyes", &[0x80, 0x1D]);
        m.insert("HalfCloseEyes", &[0x80, 0x1E]);
        m.insert("Wink", &[0x80, 0x1F]);
        m.insert("Tact", &[0x80, 0x20]);
        m.insert("ToggleRed", &[0x80, 0x21]);
        m.insert("Item", &[0x80, 0x22]);
        m.insert("SetName", &[0x80, 0x23]);
        m.insert("ToggleColorInvert", &[0x80, 0x25]);
        m.insert("#", &[0x23]); // ASCII #
        m
    })
}

pub struct Config {
    pub auto_newline: bool,
    pub auto_insert_x: bool,
    pub strip_leading_whitespace: bool,
    pub strip_trailing_whitespace: bool,
    pub utf8: bool,
}

impl Config {
    fn new(utf8: bool) -> Self {
        Self {
            auto_newline: false,
            auto_insert_x: false,
            strip_leading_whitespace: true,
            strip_trailing_whitespace: true,
            utf8,
        }
    }
}

enum Item {
    Literal(char),
    Control(String),
}

pub fn parse_text(
    text: &String,
    definitions: &HashMap<String, Vec<u8>>,
    cfg: &Config,
) -> Result<Vec<u8>> {
    let mut c = {
        match (cfg.strip_leading_whitespace, cfg.strip_trailing_whitespace) {
            (true, true) => text.trim().chars(),
            (true, false) => text.trim_start().chars(),
            (false, true) => text.trim_end().chars(),
            (false, false) => text.chars(),
        }
    };

    let mut result: Vec<u8> = Vec::new();

    let explicit_x = loop {
        match chomp_first(c.by_ref(), cfg)? {
            None => break false,
            Some(Item::Literal(c)) => write_char(c, cfg.utf8, &mut result)?,
            Some(Item::Control(s)) => {
                match handle_control_code(s, definitions)? {
                    ControlAction::Data(bytes) => result.extend(bytes.iter()),
                    ControlAction::End => {
                        result.push(0);
                        break true;
                    }
                }
            }
        }
    };

    match (explicit_x, cfg.auto_insert_x) {
        (false, true) => result.push(0),
        (false, false) => bail!("no [X] found and --auto-insert-x not set"),
        (true, _) => {
            if c.any(|c| !c.is_whitespace()) {
                bail!("found junk after [X] at end of input")
            }
        }
    }

    Ok(result)
}

pub fn parse_definition(
    line: String,
    definitions: &HashMap<String, Vec<u8>>,
    utf8: bool,
) -> Result<Option<(String, Vec<u8>)>> {
    if line.chars().all(char::is_whitespace) {
        return Ok(None);
    }

    let mut c = line.trim().chars();

    match c.next() {
        // should be impossible
        None => return Ok(None),
        Some('[') => (),
        Some('=') => {
            bail!("no LHS in definition line (hint: definition lines take the form `[A] = B`)")
        }
        Some(_) => {
            bail!("the LHS of a definition should be enclosed with brackets")
        }
    };

    let mut name = String::new();

    loop {
        match c.next() {
            None => bail!("found unclosed definition `[{0}`", name),
            Some(']') => break,
            Some(c) => {
                if c.is_whitespace() {
                    bail!("the names of definitions should not contain whitespace")
                }
                name.push(c)
            }
        }
    }

    let name = name;

    if default_control_codes().contains_key(name.as_str()) {
        bail!("cannot create definition [{0}] because it is an existing control code", name);
    }

    loop {
        match c.next() {
            None => break,
            Some(c) => {
                if !c.is_whitespace() {
                    break;
                }
            }
        }
    }

    let rhs = c.collect::<String>();
    let mut result = Vec::new();

    expand_rhs(rhs, definitions, utf8, &mut result).map_err(|e| {
        e.context(format!("while parsing definition of [{0}],", name))
    })?;

    Ok(Some((name, result)))
}

fn write_char(c: char, utf8: bool, out: &mut Vec<u8>) -> Result<()> {
    if !c.is_ascii() && !utf8 {
        bail!("found non-ascii utf-8 codepoint `{0}` without --utf8", c)
    }

    let mut buf: Vec<u8> = vec![0; c.len_utf8()];
    c.encode_utf8(&mut buf);
    out.extend(buf.iter());
    Ok(())
}

fn chomp_first<I>(mut xs: I, cfg: &Config) -> Result<Option<Item>>
where
    I: Iterator<Item = char>,
{
    match xs.next() {
        None => Ok(None),
        Some('[') => chomp_command(xs),
        Some('\n') => {
            if cfg.auto_newline {
                Ok(Some(Item::Control("NL".to_owned())))
            } else {
                chomp_first(xs, cfg)
            }
        }
        Some('#') => eat_comment(xs, cfg),
        Some(c) => Ok(Some(Item::Literal(c))),
    }
}

fn chomp_first_<I>(xs: I, utf8: bool) -> Result<Option<Item>>
where
    I: Iterator<Item = char>,
{
    chomp_first(xs, &Config::new(utf8))
}

fn chomp_command<I>(mut xs: I) -> Result<Option<Item>>
where
    I: Iterator<Item = char>,
{
    let mut result = String::new();
    loop {
        match xs.next() {
            None => {
                bail!(
                    "reached end of input while parsing control code or macro (found `[{0}`)",
                    result
                )
            }
            Some('\n') => {
                bail!("newline in the middle of control code or macro (found `[{0}`)", result)
            }
            Some(']') => return Ok(Some(Item::Control(result))),
            Some(c) => result.push(c),
        }
    }
}

fn eat_comment<I>(mut xs: I, cfg: &Config) -> Result<Option<Item>>
where
    I: Iterator<Item = char>,
{
    loop {
        match xs.next() {
            None => return Ok(None),
            Some('\n') => {
                if cfg.auto_newline {
                    return Ok(Some(Item::Control("NL".to_owned())));
                } else {
                    return chomp_first(xs, cfg);
                }
            }
            _ => (),
        }
    }
}

enum ControlAction {
    End,
    Data(Vec<u8>),
}

// CR-someday cam: There's a good bit of repeated structure between this and
// the chomp loop in parse_text, but because they involve outer control flow,
// it's a bit irritating to actually factor out.
fn expand_rhs(
    rhs: String,
    definitions: &HashMap<String, Vec<u8>>,
    utf8: bool,
    out: &mut Vec<u8>,
) -> Result<()> {
    let mut c = rhs.chars();

    loop {
        match chomp_first_(c.by_ref(), utf8)? {
            None => return Ok(()),
            Some(Item::Literal(c)) => write_char(c, utf8, out)?,
            Some(Item::Control(code)) => {
                if !definitions.contains_key(&code) {
                    bail!(
                        "unknown control code `{0}`\n\t(hint: definitions can only refer to previous definitions and cannot be recursive)",
                        code
                    );
                }
                match handle_control_code(code, definitions)? {
                    ControlAction::End => {
                        bail!("definitions should not contain [X]")
                    }
                    ControlAction::Data(target) => out.extend(target),
                }
            }
        }
    }
}

// Invariant: [definitions] and [default_control_codes] cannot overlap keys
fn expand_control(
    code: String,
    definitions: &HashMap<String, Vec<u8>>,
) -> Result<Vec<u8>> {
    match default_control_codes().get(code.as_str()) {
        None => (),
        Some(ns) => return Ok(ns.to_vec()),
    };

    match definitions.get(&code) {
        None => bail!("unknown control code {0}", code),
        Some(expansion) => Ok(expansion.clone()),
    }
}

fn handle_control_code(
    code: String,
    definitions: &HashMap<String, Vec<u8>>,
) -> Result<ControlAction> {
    if code == "X" {
        return Ok(ControlAction::End);
    }

    match code.chars().next() {
        None => bail!("found empty control code"),
        Some(c) => {
            if c.is_ascii_digit() {
                Ok(ControlAction::Data(vec![parse_number(code)?]))
            } else {
                Ok(ControlAction::Data(expand_control(code, definitions)?))
            }
        }
    }
}

fn parse_number(xs: String) -> Result<u8> {
    (if xs.starts_with("0x") {
        xs[2..].parse::<u8>()
    } else {
        xs.parse::<u8>()
    })
    .map_err(|_| anyhow!("found invalid number `[{0}]` in control code", xs))
}
