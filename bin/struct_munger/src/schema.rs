use std::{collections::HashMap, ops::Range};

use log;
use toml_edit::{InlineTable, Item, Table, TableLike, Value};

// CR cam: Validation

pub struct Schema {
    pub base_offset: usize,
    pub entry_size: usize,
    #[allow(dead_code)]
    num_entries: usize,
    fields: HashMap<String, Field>,
}

pub struct Field {
    pub offset: usize,
    pub size: usize,
    pub kind: FieldKind,
}

// CR cam: pointers
pub enum FieldKind {
    Single(Option<HashMap<String, i64>>),
    Bitfield(HashMap<String, Field>),
    List(HashMap<String, Field>),
}

type SpannedErrorMsg = (String, Option<Range<usize>>);

impl Schema {
    pub fn fields(&self) -> &HashMap<String, Field> {
        &self.fields
    }

    pub fn parse(toplevel: &Table, skip_unknowns: bool) -> Result<Self, SpannedErrorMsg> {
        let mut metadata = Err(("schema is missing metadata table".to_string(), None));
        let mut fields = HashMap::new();

        for (key, item) in toplevel.iter() {
            match (key, item) {
                ("metadata", Item::Table(t)) => metadata = Ok(Metadata::parse(t)?),
                ("metadata", Item::Value(Value::InlineTable(t))) => {
                    metadata = Ok(Metadata::parse(t)?)
                }
                ("metadata", v) => {
                    return Err(("`metadata` must be a table".to_string(), v.span()))
                }
                (k, v) => {
                    if skip_unknowns && k.to_lowercase().contains("unknown") {
                        log::warn!("ignoring field `{}` as it is marked unknown", k);
                        continue;
                    }
                    if fields.contains_key(key) {
                        return Err(("key `{}` appears twice in schema".to_string(), v.span()));
                    }

                    fields.insert(k.to_string(), Field::parse(k, v, skip_unknowns)?);
                }
            }
        }

        Ok(metadata?.to_schema(fields))
    }
}

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
        let mut base_offset = Err(("metadata table missing key `base_offset`".to_string(), None));
        let mut entry_size = Err(("metadata table missing key `entry_size`".to_string(), None));
        let mut num_entries = Err(("metadata table missing key `num_entries`".to_string(), None));

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
                ("num_entries" | "size" | "count" | "num", v) => extract_nonnegative_integer(
                    v,
                    "number of entries must be a non-negative number",
                    &mut num_entries,
                )?,

                _ => log::warn!("found extraneous key {} in `metadata` table", key),
            }
        }

        Ok(Self {
            base_offset: base_offset?,
            entry_size: entry_size?,
            num_entries: num_entries?,
        })
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
                    format!("field `{}` seems to have no attached data", path),
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

        impl Kind {
            fn fmt(self) -> &'static str {
                match self {
                    Self::Single => "single",
                    Self::Bitfield => "bitfield",
                    Self::List => "list",
                }
            }
        }

        let path = path.as_ref();

        let bad_kind_msg = format!(
            "`{}.kind` must be one of `\"bitfield\"`, `\"list\"` or `\"single\"`",
            path
        );

        let mut kind = None;
        let mut offset = Err((format!("field `{}` has no offset", path), tbl.span()));
        let mut size = Err((format!("field `{}` has no size", path), tbl.span()));

        let mut subfields = HashMap::new();

        let mut variants = None;

        for (k, v) in tbl.iter() {
            if skip_unknowns && k.to_lowercase().contains("unknown") {
                log::warn!("ignoring field `{}` as it is marked unknown", k);
                continue;
            }

            match (k, v) {
                ("kind", Item::Value(Value::String(s))) => match s.value().as_ref() {
                    "bitfield" => kind = Some(Kind::Bitfield),
                    "list" => kind = Some(Kind::List),
                    "single" => kind = Some(Kind::Single),
                    _ => return Err((bad_kind_msg, s.span())),
                },
                ("kind", v) => return Err((bad_kind_msg, v.span())),
                ("offset" | "index", v) => {
                    let msg = format!("`{}.offset` must be a non-negative integer", path);
                    extract_nonnegative_integer(v, msg, &mut offset)?
                }
                ("size", v) => {
                    let msg = format!("`{}.size` must be a non-negative integer", path);
                    extract_nonnegative_integer(v, msg, &mut size)?
                }
                ("variants", Item::Value(Value::InlineTable(kvs))) => {
                    variants = Some(Self::build_variants_from_table(
                        format!("{}.variants", path),
                        kvs,
                    )?)
                }
                ("variants", Item::Table(kvs)) => {
                    variants = Some(Self::build_variants_from_table(
                        format!("{}.variants", path),
                        kvs,
                    )?)
                }
                ("variants", v) => {
                    let msg = format!("`{}.variants must be a table`", path);
                    return Err((msg, v.span()));
                }
                (k, v) => {
                    subfields.insert(
                        k.to_string(),
                        Self::parse(format!("{}.{}", path, k), v, skip_unknowns)?,
                    );
                }
            }
        }

        let offset = offset?;
        let size = size?;

        let kind = match (kind, subfields.is_empty(), variants.is_some()) {
            (None, true, _) => FieldKind::Single(variants),
            (None, false, false) => FieldKind::List(subfields),
            (None, false, true) => {
                let msg = format!("`{}` should not have both subfields and variants", path);
                return Err((msg, tbl.span()));
            }
            (Some(Kind::List), _, false) => FieldKind::List(subfields),
            (Some(Kind::Bitfield), _, false) => FieldKind::Bitfield(subfields),
            (Some(kd @ (Kind::List | Kind::Bitfield)), _, true) => {
                let msg = format!(
                    "`{}` is declared to have kind `{}`, which cannot have variants",
                    path,
                    kd.fmt()
                );
                return Err((msg, tbl.span()));
            }
            (Some(Kind::Single), true, _) => FieldKind::Single(variants),
            (Some(Kind::Single), false, _) => {
                let msg = format!(
                    "`{}` has `kind = single`, so it should not have subfields",
                    path
                );
                return Err((msg, tbl.span()));
            }
        };

        Ok(Self { offset, size, kind })
    }

    fn build_variants_from_table(
        path: impl AsRef<str>,
        tbl: &impl TableLike,
    ) -> Result<HashMap<String, i64>, SpannedErrorMsg> {
        let mut result = HashMap::new();
        let path = path.as_ref();

        for (k, v) in tbl.iter() {
            match v {
                Item::Value(Value::Integer(i)) => {
                    result.insert(k.to_string(), *i.value());
                }
                _ => {
                    let msg = format!("`{}.{}` must be an integer", path, k);
                    return Err((msg, v.span()));
                }
            }
        }

        Ok(result)
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
