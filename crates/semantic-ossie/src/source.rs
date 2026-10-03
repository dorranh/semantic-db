use crate::ImportError;
use semantic_catalog::{SourceArchive, SourceNode, SourceRef, SourceSpan, SourceValue};
use serde_saphyr::granit_parser::{Event, Parser, ScalarStyle, Span};
use std::collections::{BTreeMap, VecDeque};

const MAX_NODES: usize = 1_000_000;
const MAX_DEPTH: usize = 128;

pub(crate) fn normalize(text: &str, archive: &SourceArchive) -> Result<SourceNode, ImportError> {
    if text.len() > 16 * 1024 * 1024 {
        return Err(error("source byte budget exceeded"));
    }
    let mut events = VecDeque::new();
    for event in Parser::new_from_str(text) {
        let (event, span) = event.map_err(|_| error("invalid source syntax"))?;
        if matches!(
            event,
            Event::StreamStart
                | Event::StreamEnd
                | Event::DocumentStart(..)
                | Event::DocumentEnd
                | Event::Comment(..)
        ) {
            continue;
        }
        if events.len() >= MAX_NODES {
            return Err(error("source event budget exceeded"));
        }
        events.push_back((event, span));
    }
    let mut reader = Reader {
        events,
        anchors: BTreeMap::new(),
        revision: archive.revision(),
        remaining: MAX_NODES,
    };
    let root = reader.node("", 0)?;
    if !reader.events.is_empty() {
        return Err(error("expected exactly one source document"));
    }
    Ok(root)
}
fn error(message: &str) -> ImportError {
    ImportError::diagnostic("source_normalization", "/", message)
}
struct Reader<'a> {
    events: VecDeque<(Event<'a>, Span)>,
    anchors: BTreeMap<usize, (SourceNode, usize)>,
    revision: &'a str,
    remaining: usize,
}
impl Reader<'_> {
    fn source(&self, path: &str, span: Span) -> Result<SourceRef, ImportError> {
        Ok(SourceRef {
            artifact_revision: self.revision.into(),
            path: path.into(),
            span: Some(SourceSpan {
                start_byte: span
                    .start
                    .byte_offset()
                    .ok_or_else(|| error("missing byte offset"))?,
                end_byte: span
                    .end
                    .byte_offset()
                    .ok_or_else(|| error("missing byte offset"))?,
                line: span.start.line(),
                column: span.start.col(),
            }),
        })
    }
    fn node(&mut self, path: &str, depth: usize) -> Result<SourceNode, ImportError> {
        if depth > MAX_DEPTH || self.remaining == 0 {
            return Err(error("source expansion budget exceeded"));
        }
        self.remaining -= 1;
        let start_remaining = self.remaining;
        let (event, mut span) = self
            .events
            .pop_front()
            .ok_or_else(|| error("incomplete source"))?;
        let (value, anchor, tag) = match event {
            Event::Scalar(value, style, anchor, tag) => {
                let core = tag.as_ref().and_then(|t| {
                    if t.handle() == "tag:yaml.org,2002:" {
                        Some(t.suffix())
                    } else {
                        None
                    }
                });
                let parsed = if style == ScalarStyle::Plain && core != Some("str") {
                    serde_saphyr::from_str_with_options::<serde_json::Value>(
                        &value,
                        serde_saphyr::options! {strict_booleans: true},
                    )
                    .ok()
                } else {
                    None
                };
                let scalar = match (core, parsed) {
                    (None, _) if style == ScalarStyle::Plain && value.is_empty() => {
                        SourceValue::Null
                    }
                    (Some("null"), _) | (_, Some(serde_json::Value::Null)) => SourceValue::Null,
                    (Some("bool"), _) => SourceValue::Boolean(value.eq_ignore_ascii_case("true")),
                    (_, Some(serde_json::Value::Bool(v))) => SourceValue::Boolean(v),
                    (Some("int" | "float"), _) | (_, Some(serde_json::Value::Number(_))) => {
                        SourceValue::Number(value.into_owned())
                    }
                    _ => SourceValue::String(value.into_owned()),
                };
                (
                    scalar,
                    anchor,
                    tag.map(|t| format!("{}{}", t.handle(), t.suffix())),
                )
            }
            Event::SequenceStart(_, anchor, tag) => {
                let mut values = Vec::new();
                while !matches!(self.events.front(), Some((Event::SequenceEnd, _))) {
                    values.push(self.node(&format!("{path}/{}", values.len()), depth + 1)?);
                }
                span.end = self.events.pop_front().expect("sequence end").1.end;
                (
                    SourceValue::Sequence(values),
                    anchor,
                    tag.map(|t| format!("{}{}", t.handle(), t.suffix())),
                )
            }
            Event::MappingStart(_, anchor, tag) => {
                let mut values = BTreeMap::new();
                let mut inherited = BTreeMap::new();
                while !matches!(self.events.front(), Some((Event::MappingEnd, _))) {
                    let (key, merge, key_span) = match self.events.pop_front() {
                        Some((Event::Scalar(key, style, _, tag), span)) => {
                            let merge_tag = tag.as_ref().is_some_and(|tag| {
                                tag.handle() == "tag:yaml.org,2002:" && tag.suffix() == "merge"
                            });
                            let merge = key == "<<"
                                && (merge_tag || (tag.is_none() && style == ScalarStyle::Plain));
                            (key.into_owned(), merge, span)
                        }
                        _ => return Err(error("expected a scalar object key")),
                    };
                    let pointer = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                    let node = self.node(&pointer, depth + 1)?;
                    if merge {
                        let nodes = match node.value {
                            SourceValue::Sequence(nodes) => nodes,
                            _ => vec![node],
                        };
                        for node in nodes {
                            let SourceValue::Mapping(mapping) = node.value else {
                                return Err(error("merge keys require mappings"));
                            };
                            for (key, mut value) in mapping {
                                let pointer =
                                    format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                                add_alias_origin(
                                    &mut value,
                                    &self.source(&pointer, key_span)?,
                                    &pointer,
                                );
                                inherited.entry(key).or_insert(value);
                            }
                        }
                    } else if values.insert(key, node).is_some() {
                        return Err(error("duplicate object key"));
                    }
                }
                for (key, value) in inherited {
                    values.entry(key).or_insert(value);
                }
                span.end = self.events.pop_front().expect("mapping end").1.end;
                (
                    SourceValue::Mapping(values),
                    anchor,
                    tag.map(|t| format!("{}{}", t.handle(), t.suffix())),
                )
            }
            Event::Alias(id) => {
                let (original, count) = self
                    .anchors
                    .get(&id)
                    .ok_or_else(|| error("recursive or unknown source alias"))?;
                if *count > self.remaining {
                    return Err(error("source alias budget exceeded"));
                }
                self.remaining -= *count;
                let mut node = original.clone();
                add_alias_origin(&mut node, &self.source(path, span)?, path);
                return Ok(node);
            }
            _ => return Err(error("unexpected source event")),
        };
        let node = SourceNode {
            value,
            tag,
            origins: vec![self.source(path, span)?],
        };
        if anchor != 0 {
            self.anchors
                .insert(anchor, (node.clone(), start_remaining - self.remaining + 1));
        }
        Ok(node)
    }
}
fn add_alias_origin(node: &mut SourceNode, origin: &SourceRef, path: &str) {
    let mut origin = origin.clone();
    origin.path = path.into();
    node.origins.push(origin.clone());
    match &mut node.value {
        SourceValue::Mapping(values) => {
            for (key, child) in values {
                add_alias_origin(
                    child,
                    &origin,
                    &format!("{path}/{}", key.replace('~', "~0").replace('/', "~1")),
                );
            }
        }
        SourceValue::Sequence(values) => {
            for (i, child) in values.iter_mut().enumerate() {
                add_alias_origin(child, &origin, &format!("{path}/{i}"));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(text: &str) -> SourceNode {
        normalize(
            text,
            &SourceArchive::new(text.as_bytes(), "yaml", "1.2", "test"),
        )
        .unwrap()
    }
    #[test]
    fn exact_numbers_absence_null_false_empty_and_prose_survive() {
        let node = parse(
            "decimal: 0.12345678901234567890123456789\nnull: null\nfalse: false\nempty: ''\nprose: 'No status history'\n",
        );
        assert_eq!(
            node.pointer("/decimal").unwrap().value,
            SourceValue::Number("0.12345678901234567890123456789".into())
        );
        assert!(node.pointer("/missing").is_none());
        assert_eq!(node.pointer("/null").unwrap().value, SourceValue::Null);
        assert_eq!(
            node.pointer("/false").unwrap().value,
            SourceValue::Boolean(false)
        );
        assert_eq!(
            node.pointer("/empty").unwrap().value,
            SourceValue::String("".into())
        );
        assert_eq!(
            node.pointer("/prose").unwrap().value,
            SourceValue::String("No status history".into())
        );
    }
    #[test]
    fn comments_unicode_aliases_and_pointer_escaping_preserve_provenance() {
        let a = parse("a: &anchor {é/a: '𝄞'}\nb: *anchor\n");
        let b = parse("# comment\na: &anchor {é/a: '𝄞'}\nb: *anchor\n");
        assert_eq!(a.semantic_revision(), b.semantic_revision());
        let value = a.pointer("/b/é~1a").unwrap();
        assert_eq!(value.origins.len(), 2);
        assert_eq!(value.origins[0].path, "/a/é~1a");
        assert_eq!(value.origins[1].path, "/b/é~1a");
        assert_ne!(
            a.origins[0].artifact_revision,
            b.origins[0].artifact_revision
        );
        assert!(
            value.origins[0].span.as_ref().unwrap().end_byte
                > value.origins[0].span.as_ref().unwrap().start_byte
        );
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;
    #[test]
    fn yaml_merges_preserve_precedence_and_effective_source_paths() {
        let text = "a: &a {amount: 0.123456789012345678901, enabled: false}\nb: &b {amount: 3, extra: null}\nc: {<<: [*a, *b], enabled: true, blank:}\n";
        let archive = SourceArchive::new(text.as_bytes(), "yaml", "1.2", "test");
        let node = normalize(text, &archive).unwrap();
        assert_eq!(
            node.pointer("/c/amount").unwrap().value,
            SourceValue::Number("0.123456789012345678901".into())
        );
        assert_eq!(
            node.pointer("/c/enabled").unwrap().value,
            SourceValue::Boolean(true)
        );
        assert_eq!(node.pointer("/c/extra").unwrap().value, SourceValue::Null);
        assert_eq!(node.pointer("/c/blank").unwrap().value, SourceValue::Null);
        assert_eq!(
            node.pointer("/c/amount")
                .unwrap()
                .origins
                .last()
                .unwrap()
                .path,
            "/c/amount"
        );
        assert!(node.pointer("/c/<<").is_none());
    }
}
