use std::{
    fs,
    ops::Range,
    path::{Path, PathBuf},
};

use ariadne::{Label, ReportKind};
use log;
use toml_edit::{ImDocument, Item, Value};

use crate::{
    entry::{self, AtomKind, Sink, ValueOut},
    schema::Schema,
};

type Report<'a> = ariadne::Report<'a, (String, Range<usize>)>;
type ReportBuilder<'a> = ariadne::ReportBuilder<'a, (String, Range<usize>)>;

pub struct Config {
    pub skip_unknowns: bool,
}

pub fn handle_document<'a, 'b, 'any, S>(
    source: impl AsRef<Path>,
    doc: ImDocument<S>,
    schema: Option<&'a Schema>,
    config: &'b Config,
) -> Result<Vec<String>, Vec<Report<'any>>> {
    let source = format!("{}", source.as_ref().display());
    let local_schema;

    let schema = match (schema, doc.get("schema")) {
        (Some(s), None) => s,
        (Some(s), Some(_)) => {
            log::info!(
                "ignoring schema specified in document in favor of \
                       explicit schema given"
            );
            s
        }
        (None, Some(Item::Value(Value::String(path)))) => {
            local_schema = find_schema(source.clone(), path.value(), config.skip_unknowns)
                .map_err(|report| {
                    vec![(if let Some(span) = path.span() {
                        report.with_label(
                            Label::new((source.clone(), span))
                                .with_message(format!("when trying to load schema `{}`", path)),
                        )
                    } else {
                        report
                    })
                    .finish()]
                })?;
            &local_schema
        }
        (None, Some(Item::Table(tbl))) => match Schema::parse(tbl, config.skip_unknowns) {
            Ok(s) => {
                local_schema = s;
                &local_schema
            }
            Err((msg, span)) => {
                let mut report =
                    Report::build(ReportKind::Error, source.clone(), 0).with_message(msg);

                if let Some(span) = span {
                    report = report.with_label(Label::new((source.clone(), span)));
                }

                let report = report;

                return Err(vec![report.finish()]);
            }
        },
        (None, Some(_)) => {
            let span = doc.key("schema").unwrap().span();

            let mut report = Report::build(ReportKind::Error, source.clone(), 0)
                .with_message("field `schema` should be a table or string");

            if let Some(span) = span {
                report = report
                    .with_label(Label::new((source.clone(), span)).with_message("(defined here)"));
            }

            let report = report;

            return Err(vec![report.finish()]);
        }
        (None, None) => {
            return Err(vec![Report::build(ReportKind::Error, source.clone(), 0)
                .with_message("document does not specify a schema and none was provided")
                .finish()])
        }
    };

    let mut writer = StructWriter::from_schema(&schema);
    let mut errs = Vec::new();

    for (k, v) in doc.iter() {
        if k == "schema" {
            continue;
        }

        writer.print(format!("// entry for {}", k));
        let mut sink = writer.as_sink(k.to_string());

        match v {
            Item::Value(Value::InlineTable(ts)) => {
                if let Err((msg, span)) = entry::visit(ts, schema.fields(), &mut sink) {
                    errs.push(make_error_report(msg, span, source.clone()));
                }
            }
            Item::Table(ts) => {
                if let Err((msg, span)) = entry::visit(ts, schema.fields(), &mut sink) {
                    errs.push(make_error_report(msg, span, source.clone()));
                }
            }
            _ => errs.push({
                let mut report =
                    Report::build(ReportKind::Error, source.clone(), 0).with_message(format!(
                        "don't know what to do with top-level entry `{}` \
                                          that is not a table",
                        k
                    ));
                if let Some(span) = v.span() {
                    report = report.with_label(
                        Label::new((source.clone(), span)).with_message("(defined here)"),
                    );
                }
                report.finish()
            }),
        }
    }

    if errs.is_empty() {
        Ok(writer.destruct())
    } else {
        Err(errs)
    }
}

pub fn load_schema<'a>(
    path: impl AsRef<Path>,
    skip_unknowns: bool,
) -> Result<Schema, ReportBuilder<'a>> {
    let path = path.as_ref();
    let path_string = format!("{}", path.display());

    let text = fs::read_to_string(path).map_err({
        let path_string = path_string.clone();

        |err| Report::build(ReportKind::Error, path_string, 0).with_message(format!("{}", err))
    })?;

    let doc = ImDocument::parse(text).map_err({
        let path_string = path_string.clone();
        |err| {
            let mut result = Report::build(ReportKind::Error, path_string.clone(), 0)
                .with_message(err.message());

            if let Some(span) = err.span() {
                result = result.with_label(Label::new((path_string, span)).with_message("(here)"));
            }

            result
        }
    })?;

    Schema::parse(doc.as_table(), skip_unknowns).map_err(|(msg, span)| {
        let mut result = Report::build(ReportKind::Error, path_string.clone(), 0).with_message(msg);

        if let Some(span) = span {
            result =
                result.with_label(Label::new((path_string, span)).with_message("(defined here)"));
        }

        result
    })
}

fn find_schema<'a>(
    source: impl AsRef<Path>,
    p: &str,
    skip_unknowns: bool,
) -> Result<Schema, ReportBuilder<'a>> {
    let source: PathBuf = source.as_ref().to_owned();
    let path = source.with_file_name(format!("{}.schema.toml", p));
    load_schema(path, skip_unknowns)
}

fn make_error_report<'a>(msg: String, span: Option<Range<usize>>, source: String) -> Report<'a> {
    let mut report = Report::build(ReportKind::Error, source.clone(), 0).with_message(msg);

    if let Some(span) = span {
        report = report.with_label(Label::new((source, span)));
    }

    report.finish()
}

fn header() -> Vec<String> {
    vec!["// generated by structmunger", "", "PUSH", ""]
        .into_iter()
        .map(str::to_string)
        .collect()
}

struct StructWriter {
    data: Vec<String>,
    // CR-soon cam: allow this to be a definition
    base: String,
    entry_size: usize,
}

impl StructWriter {
    pub fn print(&mut self, s: impl AsRef<str>) {
        self.data.push(s.as_ref().to_string())
    }

    pub fn from_schema(schema: &Schema) -> Self {
        Self {
            data: header(),
            base: format!("{:#X}", schema.base_offset),
            entry_size: schema.entry_size,
        }
    }

    pub fn destruct(mut self) -> Vec<String> {
        self.print("");
        self.print("POP");
        self.data
    }

    pub fn as_sink<'a>(&'a mut self, index: String) -> StructWriterSink<'a> {
        self.print(format!(
            "ORG ({})+(({})*({}))",
            self.base, index, self.entry_size
        ));

        StructWriterSink {
            owner: self,
            current_index: index,
            current_offset: 0,
        }
    }
}

struct StructWriterSink<'a> {
    owner: &'a mut StructWriter,
    current_index: String,
    current_offset: usize,
}

impl StructWriterSink<'_> {
    pub fn print(&mut self, s: impl AsRef<str>) {
        self.owner.print(s)
    }
}

impl Sink for StructWriterSink<'_> {
    fn write(&mut self, offset: usize, kind: AtomKind, data: impl ValueOut) {
        if offset != self.current_offset {
            self.print(format!(
                "ORG ({})+(({})*({}))+({})",
                self.owner.base, self.current_index, self.owner.entry_size, offset
            ))
        }
        self.print(format!("{} {}", kind, data.reify()));
        self.current_offset = offset + kind.to_usize();
    }
}
