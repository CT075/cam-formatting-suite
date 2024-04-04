use std::{collections::HashMap, fmt, ops::Range};

use toml_edit::{Item, TableLike, Value};

use crate::schema::{Field, FieldKind};

type SpannedErrorMsg = (String, Option<Range<usize>>);

#[derive(Copy, Clone)]
pub enum AtomKind {
    Byte,
    Short,
    Word,
}

impl AtomKind {
    pub fn from_byte_size(
        n: usize,
        fail_msg: impl AsRef<str>,
        src_loc: Option<Range<usize>>,
    ) -> Result<Self, SpannedErrorMsg> {
        match n {
            1 => Ok(Self::Byte),
            2 => Ok(Self::Short),
            4 => Ok(Self::Word),
            _ => Err((fail_msg.as_ref().to_string(), src_loc)),
        }
    }

    pub fn to_usize(&self) -> usize {
        match *self {
            Self::Byte => 1,
            Self::Short => 2,
            Self::Word => 4,
        }
    }

    pub fn value_valid(self, i: i64) -> bool {
        match self {
            Self::Word => true,
            Self::Short => -32768 <= i && i <= 32767,
            Self::Byte => -128 <= i && i <= 255,
        }
    }
}

impl fmt::Display for AtomKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Self::Byte => write!(f, "BYTE"),
            Self::Short => write!(f, "SHORT"),
            Self::Word => write!(f, "WORD"),
        }
    }
}

pub trait ValueOut {
    fn reify(&self) -> String;
}

impl ValueOut for i64 {
    fn reify(&self) -> String {
        format!("({})", self)
    }
}

impl ValueOut for &str {
    fn reify(&self) -> String {
        self.to_string()
    }
}

impl ValueOut for &String {
    fn reify(&self) -> String {
        self.to_string()
    }
}

pub trait Sink {
    fn write(&mut self, offset: usize, kind: AtomKind, data: impl ValueOut);
}

pub fn visit(
    tbl: &impl TableLike,
    fields: &HashMap<String, Field>,
    sink: &mut impl Sink,
) -> Result<(), SpannedErrorMsg> {
    for (k, v) in tbl.iter() {
        match fields.get(k) {
            None => {
                // This `unwrap` cannot fail, because we already know `k` is in
                // the table
                let k = tbl.key(k).unwrap();
                return Err((
                    format!("no entry for key `{}` in schema", k),
                    k.span(),
                ));
            }
            Some(field) => match (v, &field.kind) {
                (Item::Value(Value::Integer(i)), _) => {
                    let atomkind = AtomKind::from_byte_size(
                        field.size,
                        make_atom_size_msg(k),
                        i.span(),
                    )?;
                    if !atomkind.value_valid(*i.value()) {
                        return Err((
                            format!(
                                "the field `{}` has size `{}`, which is too \
                                small to fit the value `{}`",
                                k, field.size, i,
                            ),
                            i.span(),
                        ));
                    }
                    sink.write(field.offset, atomkind, *i.value())
                }
                (Item::Value(Value::String(s)), FieldKind::Single(vs)) => {
                    if let Some(variants) = vs {
                        if let Some(i) = variants.get(s.value()) {
                            sink.write(
                                field.offset,
                                AtomKind::from_byte_size(
                                    field.size,
                                    make_atom_size_msg(k),
                                    s.span(),
                                )?,
                                *i,
                            );
                            continue;
                        }
                    }

                    sink.write(
                        field.offset,
                        AtomKind::from_byte_size(
                            field.size,
                            make_atom_size_msg(k),
                            s.span(),
                        )?,
                        s.value(),
                    );
                }
                (Item::Value(Value::String(s)), _) => sink.write(
                    field.offset,
                    AtomKind::from_byte_size(
                        field.size,
                        make_atom_size_msg(k),
                        s.span(),
                    )?,
                    s.value(),
                ),
                (_, FieldKind::Single(_)) => {
                    return Err((
                        format!(
                            "don't know what to do with field `{}` \
                                (expected an integer or string)",
                            k
                        ),
                        v.span(),
                    ))
                }
                (
                    Item::Value(Value::InlineTable(items)),
                    FieldKind::List(subfields),
                ) => visit(items, subfields, sink)?,
                (Item::Table(items), FieldKind::List(subfields)) => {
                    visit(items, subfields, sink)?
                }
                (_, FieldKind::List(_)) => {
                    return Err((
                        format!(
                            "don't know what to do with field `{}` \
                                (expected an integer, string, or table of \
                                 subfields)",
                            k
                        ),
                        v.span(),
                    ))
                }
                (v, FieldKind::Bitfield(_)) => {
                    return Err((
                        "inserting bitfields is not yet supported".to_string(),
                        v.span(),
                    ))
                }
            },
        }
    }

    Ok(())
}

fn make_atom_size_msg(path: impl AsRef<str>) -> String {
    format!(
        "refusing to write directly to field path `{}`, \
         because it does not have size 1, 2 or 4",
        path.as_ref()
    )
}
