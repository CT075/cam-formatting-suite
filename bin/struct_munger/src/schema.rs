use std::{collections::HashMap, ops::Range};

use log;
use toml_edit::{ImDocument, InlineTable, Item, Table, TableLike, Value};

pub struct Schema {
    base_offset: usize,
    entry_size: usize,
    num_entries: usize,
    fields: HashMap<String, Field>,
}

pub struct Field {
    header: FieldHeader,
    kind: FieldKind,
}

// CR cam: pointers
pub enum FieldKind {
    Single,
    Bitfield(HashMap<String, Field>),
    List(HashMap<String, Field>),
}

pub struct FieldHeader {
    offset: usize,
    size: usize,
}

type SpannedErrorMsg = (String, Option<Range<usize>>);

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

    fn parse(tbl: &impl TableLike) -> Result<Metadata, SpannedErrorMsg> {
        let mut base_offset =
            Err(("metadata table missing key `base_offset`".to_string(), None));
        let mut entry_size =
            Err(("metadata table missing key `entry_size`".to_string(), None));
        let mut num_entries =
            Err(("metadata table missing key `num_entries`".to_string(), None));

        for (key, item) in tbl.iter() {
            match (key, item) {
                ("base_offset" | "base", v) => {
                    let v: &Item = v;
                    extract_nonnegative_integer(
                        v,
                        "base offset must be a non-negative number",
                        &mut base_offset,
                    )?
                }
                ("entry_size", v) => extract_nonnegative_integer(
                    v,
                    "entry size must be a non-negative number",
                    &mut entry_size,
                )?,
                ("num_entries" | "size" | "count" | "num", v) => {
                    extract_nonnegative_integer(
                        v,
                        "number of entries must be a non-negative number",
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

impl Schema {
    pub fn parse<S>(
        doc: ImDocument<S>,
        skip_unknowns: bool,
    ) -> Result<Self, SpannedErrorMsg>
    where
        S: AsRef<str>,
    {
        let mut metadata =
            Err(("schema is missing metadata table".to_string(), None));
        let mut fields = HashMap::new();

        for (key, item) in doc.iter() {
            match (key, item) {
                ("metadata", Item::Table(t)) => {
                    metadata = Ok(Metadata::parse(t)?)
                }
                ("metadata", Item::Value(Value::InlineTable(t))) => {
                    metadata = Ok(Metadata::parse(t)?)
                }
                ("metadata", v) => {
                    return Err((
                        "`metadata` must be a table".to_string(),
                        v.span(),
                    ))
                }
                (k, v) => {
                    if skip_unknowns && k.to_lowercase().contains("unknown") {
                        log::warn!(
                            "ignoring field `{}` as it is marked unknown",
                            k
                        );
                        continue;
                    }
                    if fields.contains_key(key) {
                        return Err((
                            "key `{}` appears twice in schema".to_string(),
                            v.span(),
                        ));
                    }

                    fields.insert(
                        k.to_string(),
                        Field::parse(k, v, skip_unknowns)?,
                    );
                }
            }
        }

        Ok(metadata?.to_schema(fields))
    }
}

impl Field {
    pub fn parse(
        path: impl AsRef<str>,
        t: &Item,
        skip_unknowns: bool,
    ) -> Result<Self, SpannedErrorMsg> {
        let path = path.as_ref();

        match t {
            Item::None => {
                return Err((
                    format!(
                        "field `{}` seems to have no attached data",
                        path
                    ),
                    t.span(),
                ));
            }
            Item::Value(v) => match v {
                Value::InlineTable(kvs) => Self::build_field(path, kvs, skip_unknowns),
                v => {
                    return Err((
                        format!(
                            "field `{}` has value `{}`, which is not a table",
                            path,
                            v.to_string()
                        ),
                        t.span(),
                    ))
                }
            },
            Item::ArrayOfTables(_) => {
                return Err((
                    format!(
                        "field `{}` is an array of tables, but should just be a table",
                        path,
                    ),
                    t.span(),
                ))
            }
            Item::Table(t) => Self::build_field(path, t, skip_unknowns),
        }
    }

    fn build_field(
        path: impl AsRef<str>,
        tbl: &(impl TableLike + Spanned),
        skip_unknowns: bool,
    ) -> Result<Self, SpannedErrorMsg> {
        enum Kind {
            Single,
            Bitfield,
            List,
        }

        let path = path.as_ref();

        let bad_kind_msg = format!(
            "`{}.kind` must be one of `\"bitfield\"`, `\"list\"` or `\"single\"`",
            path);

        let mut kind = None;
        let mut offset =
            Err((format!("field `{}` has no offset", path), tbl.span()));
        let mut size =
            Err((format!("field `{}` has no size", path), tbl.span()));

        let mut subfields = HashMap::new();

        for (k, v) in tbl.iter() {
            if skip_unknowns && k.to_lowercase().contains("unknown") {
                log::warn!("ignoring field `{}` as it is marked unknown", k);
                continue;
            }

            match (k, v) {
                ("kind", Item::Value(Value::String(s))) => {
                    match s.value().as_ref() {
                        "bitfield" => kind = Some(Kind::Bitfield),
                        "list" => kind = Some(Kind::List),
                        "single" => kind = Some(Kind::Single),
                        _ => return Err((bad_kind_msg, s.span())),
                    }
                }
                ("kind", v) => return Err((bad_kind_msg, v.span())),
                ("offset", v) => {
                    let msg = format!(
                        "`{}.offset` must be a non-negative integer",
                        path
                    );
                    extract_nonnegative_integer(v, msg, &mut offset)?
                }
                ("size", v) => {
                    let msg = format!(
                        "`{}.size` must be a non-negative integer",
                        path
                    );
                    extract_nonnegative_integer(v, msg, &mut size)?
                }
                (k, v) => {
                    subfields.insert(
                        k.to_string(),
                        Self::parse(
                            format!("{}.{}", path, k),
                            v,
                            skip_unknowns,
                        )?,
                    );
                }
            }
        }

        let header = FieldHeader {
            offset: offset?,
            size: size?,
        };

        let kind = match (kind, subfields.is_empty()) {
            (None, true) => FieldKind::Single,
            (None, false) => FieldKind::List(subfields),
            (Some(Kind::List), _) => FieldKind::List(subfields),
            (Some(Kind::Bitfield), _) => FieldKind::Bitfield(subfields),
            (Some(Kind::Single), true) => FieldKind::Single,
            (Some(Kind::Single), false) => {
                let msg = format!(
                    "`{}` has `kind = single` but also has listed subfields",
                    path
                );
                return Err((msg, tbl.span()));
            }
        };

        Ok(Self { header, kind })
    }
}

fn extract_nonnegative_integer(
    item: &Item,
    msg: impl AsRef<str>,
    dst: &mut Result<usize, SpannedErrorMsg>,
) -> Result<(), SpannedErrorMsg> {
    if let Item::Value(Value::Integer(fi)) = item {
        let i = *fi.value();
        if i < 0 {
            return Err((msg.as_ref().to_string(), item.span()));
        }
        *dst = Ok(i as usize);
    } else {
        return Err((msg.as_ref().to_string(), item.span()));
    }
    Ok(())
}

trait Spanned {
    fn span(&self) -> Option<Range<usize>>;
}

impl Spanned for InlineTable {
    fn span(&self) -> Option<Range<usize>> {
        InlineTable::span(self)
    }
}

impl Spanned for Table {
    fn span(&self) -> Option<Range<usize>> {
        Table::span(self)
    }
}
