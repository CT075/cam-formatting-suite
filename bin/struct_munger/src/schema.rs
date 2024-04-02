use std::{collections::HashMap, ops::Range};

use log;
use toml_edit::{ImDocument, Item, Value};

pub struct Schema {
    base_offset: usize,
    entry_size: usize,
    num_entries: usize,
    fields: HashMap<String, Field>,
}

pub enum Field {
    Single(Entry),
    Bitfield(HashMap<String, Entry>),
    List(HashMap<String, Entry>),
}

// CR cam: pointers
pub struct Entry {
    offset: usize,
    size: usize,
}

trait SpannedMaybeInteger {
    fn extract_integer(&self) -> Option<i64>;
    fn span(&self) -> Option<Range<usize>>;
}

impl<'a> SpannedMaybeInteger for &'a Item {
    fn extract_integer(&self) -> Option<i64> {
        match self {
            Item::Value(v) => v.extract_integer(),
            _ => None,
        }
    }

    fn span(&self) -> Option<Range<usize>> {
        Self::span(self)
    }
}

impl<'a> SpannedMaybeInteger for &'a Value {
    fn extract_integer(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(*i.value()),
            _ => None,
        }
    }

    fn span(&self) -> Option<Range<usize>> {
        Self::span(self)
    }
}

type SpannedErrorMsg = (&'static str, Option<Range<usize>>);

struct Metadata {
    base_offset: usize,
    entry_size: usize,
    num_entries: usize,
}

impl Metadata {
    fn to_schema(self, fields: HashMap<String, Field>) -> Schema {
        let Self {
            base_offset,
            entry_size,
            num_entries,
        } = self;
        Schema {
            base_offset,
            entry_size,
            num_entries,
            fields,
        }
    }

    fn try_extract(
        v: impl SpannedMaybeInteger,
        msg: &'static str,
        dst: &mut Result<usize, SpannedErrorMsg>,
    ) -> Result<(), SpannedErrorMsg> {
        if let Some(i) = v.extract_integer() {
            if i < 0 {
                return Err((msg, v.span()));
            }
            *dst = Ok(i as usize);
        } else {
            return Err((msg, v.span()));
        }
        Ok(())
    }

    fn parse<'a, I>(
        iter: impl Iterator<Item = (&'a str, I)>,
    ) -> Result<Metadata, SpannedErrorMsg>
    where
        I: SpannedMaybeInteger,
    {
        let mut base_offset =
            Err(("metadata table missing key `base_offset`", None));
        let mut entry_size =
            Err(("metadata table missing key `entry_size`", None));
        let mut num_entries =
            Err(("metadata table missing key `num_entries`", None));

        for (key, item) in iter {
            match (key, item) {
                ("base_offset" | "base", v) => Self::try_extract(
                    v,
                    "base offset must be non-negative",
                    &mut base_offset,
                )?,
                ("entry_size", v) => Self::try_extract(
                    v,
                    "entry size must be non-negative",
                    &mut entry_size,
                )?,
                ("num_entries" | "size" | "count" | "num", v) => {
                    Self::try_extract(
                        v,
                        "number of entries must be non-negative",
                        &mut num_entries,
                    )?
                }

                _ => log::warn!(
                    "found extraneous key {} in `metadata` table",
                    key
                ),
            }
        }

        Ok(Self {
            base_offset: base_offset?,
            entry_size: entry_size?,
            num_entries: num_entries?,
        })
    }
}

impl Field {}

impl Schema {
    pub fn parse<S>(
        doc: ImDocument<S>,
        skip_unknowns: bool,
    ) -> Result<Self, SpannedErrorMsg>
    where
        S: AsRef<str>,
    {
        let mut metadata = Err(("schema is missing metadata table", None));
        let mut fields = HashMap::new();

        for (key, item) in doc.iter() {
            match (key, item) {
                ("metadata", Item::Table(t)) => {
                    metadata = Ok(Metadata::parse(t.iter())?)
                }
                ("metadata", Item::Value(Value::InlineTable(t))) => {
                    metadata = Ok(Metadata::parse(t.iter())?)
                }
                ("metadata", v) => {
                    return Err(("`metadata` must be a table", v.span()))
                }
                (k, v) => {
                    if skip_unknowns && k.to_lowercase().contains("unknown") {
                        log::warn!("ignoring unknown field `{}`", k);
                        continue;
                    }
                }
            }
        }

        Ok(metadata?.to_schema(fields))
    }
}
