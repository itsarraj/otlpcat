//! Pure normalization, formatting, and tree-building logic — no I/O.
//! Consumes the lenient [`crate::otlp`] types and produces the flat
//! span list / rendered text `main.rs` prints.

use std::fmt::Write as _;

use base64::Engine;

use crate::otlp::{EventJson, ExportTraceServiceRequest, KeyValue};

/// Decodes an OTLP id field (`traceId`/`spanId`/`parentSpanId`) to
/// lowercase hex. Real producers disagree on wire form: the official
/// OTLP/HTTP JSON convention is hex, but a generic protobuf-JSON
/// encoder (confirmed live against `opentelemetry-python`'s own
/// `MessageToJson` output) emits standard base64 instead, since `bytes`
/// fields have no OTLP-specific override in a plain protobuf-JSON
/// marshaler. Accepts either.
pub fn decode_id_to_hex(value: &serde_json::Value) -> Option<String> {
    let s = value.as_str()?;
    if !s.is_empty() && s.len() % 2 == 0 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Some(s.to_lowercase());
    }
    let decoded = base64::engine::general_purpose::STANDARD.decode(s).ok()?;
    if decoded.is_empty() {
        return None;
    }
    Some(decoded.iter().map(|b| format!("{b:02x}")).collect())
}

/// Parses a nanosecond timestamp field, accepting both the spec-correct
/// JSON string form (protobuf JSON represents 64-bit integers as
/// strings to avoid precision loss in JS number parsing) and a bare
/// JSON number, which some hand-rolled or older encoders emit anyway.
pub fn parse_nanos(value: &serde_json::Value) -> Option<u64> {
    match value {
        serde_json::Value::String(s) => s.parse().ok(),
        serde_json::Value::Number(n) => n.as_u64(),
        _ => None,
    }
}

pub fn span_kind_label(value: Option<&serde_json::Value>) -> &'static str {
    match value {
        None => "UNSPECIFIED",
        Some(serde_json::Value::String(s)) => match s.strip_prefix("SPAN_KIND_").unwrap_or(s) {
            "INTERNAL" => "INTERNAL",
            "SERVER" => "SERVER",
            "CLIENT" => "CLIENT",
            "PRODUCER" => "PRODUCER",
            "CONSUMER" => "CONSUMER",
            _ => "UNSPECIFIED",
        },
        Some(serde_json::Value::Number(n)) => match n.as_u64() {
            Some(1) => "INTERNAL",
            Some(2) => "SERVER",
            Some(3) => "CLIENT",
            Some(4) => "PRODUCER",
            Some(5) => "CONSUMER",
            _ => "UNSPECIFIED",
        },
        _ => "UNSPECIFIED",
    }
}

pub fn status_code_label(value: Option<&serde_json::Value>) -> &'static str {
    match value {
        None => "UNSET",
        Some(serde_json::Value::String(s)) => match s.strip_prefix("STATUS_CODE_").unwrap_or(s) {
            "OK" => "OK",
            "ERROR" => "ERROR",
            _ => "UNSET",
        },
        Some(serde_json::Value::Number(n)) => match n.as_u64() {
            Some(1) => "OK",
            Some(2) => "ERROR",
            _ => "UNSET",
        },
        _ => "UNSET",
    }
}

/// Renders one OTLP `AnyValue` JSON object (`{"stringValue": "..."}`
/// etc.) into a compact, human-readable string.
pub fn format_any_value(value: &serde_json::Value) -> String {
    let obj = match value.as_object() {
        Some(o) => o,
        None => return value.to_string(),
    };
    if let Some(v) = obj.get("stringValue").and_then(|v| v.as_str()) {
        return v.to_string();
    }
    if let Some(v) = obj.get("intValue") {
        return match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
    }
    if let Some(v) = obj.get("doubleValue") {
        return v.to_string();
    }
    if let Some(v) = obj.get("boolValue") {
        return v.to_string();
    }
    if let Some(v) = obj.get("bytesValue").and_then(|v| v.as_str()) {
        return format!("<{} bytes base64>", v.len());
    }
    if let Some(arr) = obj
        .get("arrayValue")
        .and_then(|v| v.get("values"))
        .and_then(|v| v.as_array())
    {
        let items: Vec<String> = arr.iter().map(format_any_value).collect();
        return format!("[{}]", items.join(", "));
    }
    if let Some(kvs) = obj
        .get("kvlistValue")
        .and_then(|v| v.get("values"))
        .and_then(|v| v.as_array())
    {
        let items: Vec<String> = kvs
            .iter()
            .filter_map(|kv| {
                let key = kv.get("key")?.as_str()?;
                let val = kv.get("value").map(format_any_value).unwrap_or_default();
                Some(format!("{key}={val}"))
            })
            .collect();
        return format!("{{{}}}", items.join(", "));
    }
    "null".to_string()
}

pub fn format_attributes(attrs: &[KeyValue]) -> Vec<(String, String)> {
    attrs
        .iter()
        .map(|kv| {
            let rendered = kv.value.as_ref().map(format_any_value).unwrap_or_default();
            (kv.key.clone(), rendered)
        })
        .collect()
}

/// Formats a nanosecond duration as a human-scaled string: µs below 1ms,
/// ms below 1s, seconds with 3 decimals otherwise.
pub fn format_duration_nanos(nanos: u64) -> String {
    if nanos < 1_000 {
        format!("{nanos}ns")
    } else if nanos < 1_000_000 {
        format!("{:.2}µs", nanos as f64 / 1_000.0)
    } else if nanos < 1_000_000_000 {
        format!("{:.2}ms", nanos as f64 / 1_000_000.0)
    } else {
        format!("{:.3}s", nanos as f64 / 1_000_000_000.0)
    }
}

#[derive(Debug, Clone)]
pub struct FlatEvent {
    pub name: String,
    pub time_nanos: Option<u64>,
    pub attributes: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct FlatSpan {
    pub service_name: Option<String>,
    pub scope_name: String,
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    pub kind: &'static str,
    pub start_nanos: u64,
    pub end_nanos: Option<u64>,
    pub attributes: Vec<(String, String)>,
    pub status_code: &'static str,
    pub status_message: Option<String>,
    pub events: Vec<FlatEvent>,
}

impl FlatSpan {
    pub fn duration_nanos(&self) -> Option<u64> {
        self.end_nanos
            .map(|end| end.saturating_sub(self.start_nanos))
    }
}

fn service_name_of(resource: &Option<crate::otlp::Resource>) -> Option<String> {
    resource
        .as_ref()?
        .attributes
        .iter()
        .find(|kv| kv.key == "service.name")
        .and_then(|kv| kv.value.as_ref())
        .and_then(|v| v.as_object())
        .and_then(|o| o.get("stringValue"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// Walks the full nested resourceSpans → scopeSpans → spans structure
/// and produces one flat, fully-normalized list — resource/scope
/// context attached to each span rather than left implicit in nesting.
pub fn flatten(doc: &ExportTraceServiceRequest) -> Vec<FlatSpan> {
    let mut out = Vec::new();
    for rs in &doc.resource_spans {
        let service_name = service_name_of(&rs.resource);
        for ss in &rs.scope_spans {
            let scope_name = ss
                .scope
                .as_ref()
                .map(|s| s.name.clone())
                .unwrap_or_default();
            for span in &ss.spans {
                let events = span.events.iter().map(flatten_event).collect();
                let trace_id = decode_id_to_hex(&span.trace_id).unwrap_or_else(|| "?".to_string());
                let span_id = decode_id_to_hex(&span.span_id).unwrap_or_else(|| "?".to_string());
                let parent_span_id = span.parent_span_id.as_ref().and_then(decode_id_to_hex);
                out.push(FlatSpan {
                    service_name: service_name.clone(),
                    scope_name: scope_name.clone(),
                    trace_id,
                    span_id,
                    parent_span_id,
                    name: span.name.clone(),
                    kind: span_kind_label(span.kind.as_ref()),
                    start_nanos: parse_nanos(&span.start_time_unix_nano).unwrap_or(0),
                    end_nanos: span.end_time_unix_nano.as_ref().and_then(parse_nanos),
                    attributes: format_attributes(&span.attributes),
                    status_code: status_code_label(
                        span.status.as_ref().and_then(|s| s.code.as_ref()),
                    ),
                    status_message: span.status.as_ref().and_then(|s| s.message.clone()),
                    events,
                });
            }
        }
    }
    out
}

fn flatten_event(event: &EventJson) -> FlatEvent {
    FlatEvent {
        name: event.name.clone(),
        time_nanos: event.time_unix_nano.as_ref().and_then(parse_nanos),
        attributes: format_attributes(&event.attributes),
    }
}

pub struct TreeNode<'a> {
    pub span: &'a FlatSpan,
    pub children: Vec<TreeNode<'a>>,
}

/// Groups flat spans by trace id, then within each trace links each
/// span under its parent by span id. A span whose parent id is absent,
/// empty, or not found among the spans in this document (e.g. the
/// parent lives in a different service's export) becomes a root — this
/// is a normal, expected case for a single-service capture of a
/// distributed trace, not an error.
pub fn build_forest(spans: &[FlatSpan]) -> Vec<(String, Vec<TreeNode<'_>>)> {
    let mut trace_ids: Vec<String> = spans.iter().map(|s| s.trace_id.clone()).collect();
    trace_ids.sort();
    trace_ids.dedup();

    trace_ids
        .into_iter()
        .map(|trace_id| {
            let trace_spans: Vec<&FlatSpan> =
                spans.iter().filter(|s| s.trace_id == trace_id).collect();
            let roots = build_tree_for_trace(&trace_spans);
            (trace_id, roots)
        })
        .collect()
}

fn build_tree_for_trace<'a>(spans: &[&'a FlatSpan]) -> Vec<TreeNode<'a>> {
    fn children_of<'a>(spans: &[&'a FlatSpan], parent_id: Option<&str>) -> Vec<TreeNode<'a>> {
        spans
            .iter()
            .filter(|s| {
                let has_parent_in_set = s
                    .parent_span_id
                    .as_deref()
                    .is_some_and(|p| spans.iter().any(|other| other.span_id == p));
                match parent_id {
                    None => s.parent_span_id.is_none() || !has_parent_in_set,
                    Some(pid) => s.parent_span_id.as_deref() == Some(pid),
                }
            })
            .map(|s| TreeNode {
                span: s,
                children: children_of(spans, Some(&s.span_id)),
            })
            .collect()
    }
    children_of(spans, None)
}

fn render_node(node: &TreeNode, prefix: &str, is_last: bool, out: &mut String) {
    let connector = if is_last { "└─ " } else { "├─ " };
    let duration = node
        .span
        .duration_nanos()
        .map(format_duration_nanos)
        .unwrap_or_else(|| "unfinished".to_string());
    let service = node.span.service_name.as_deref().unwrap_or("-");
    let _ = writeln!(
        out,
        "{prefix}{connector}{} [{}] service={} ({})",
        node.span.name, node.span.kind, service, duration
    );
    let child_prefix = format!("{prefix}{}", if is_last { "   " } else { "│  " });

    if node.span.status_code == "ERROR" {
        let msg = node
            .span
            .status_message
            .as_deref()
            .map(|m| format!(": {m}"))
            .unwrap_or_default();
        let _ = writeln!(out, "{child_prefix}STATUS=ERROR{msg}");
    }
    if !node.span.attributes.is_empty() {
        let attrs: Vec<String> = node
            .span
            .attributes
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let _ = writeln!(out, "{child_prefix}attrs: {}", attrs.join(", "));
    }
    for event in &node.span.events {
        let rel = event
            .time_nanos
            .map(|t| t.saturating_sub(node.span.start_nanos))
            .map(format_duration_nanos)
            .unwrap_or_else(|| "?".into());
        let attrs: Vec<String> = event
            .attributes
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let attrs_str = if attrs.is_empty() {
            String::new()
        } else {
            format!("  {}", attrs.join(", "))
        };
        let _ = writeln!(out, "{child_prefix}event +{rel} {}{attrs_str}", event.name);
    }

    let n = node.children.len();
    for (i, child) in node.children.iter().enumerate() {
        render_node(child, &child_prefix, i == n - 1, out);
    }
}

/// Renders every trace in `spans` as a human-readable indented tree:
/// one root heading per trace id, spans nested under their parent with
/// `tree`-style box-drawing connectors, duration, status, attributes,
/// and events.
pub fn render_trace_tree(spans: &[FlatSpan]) -> String {
    let forest = build_forest(spans);
    let mut out = String::new();
    for (trace_id, roots) in &forest {
        let _ = writeln!(out, "Trace {trace_id}");
        let n = roots.len();
        for (i, root) in roots.iter().enumerate() {
            render_node(root, "", i == n - 1, &mut out);
        }
        let _ = writeln!(out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decode_id_to_hex_accepts_lowercase_hex() {
        assert_eq!(
            decode_id_to_hex(&json!("aabbcc")),
            Some("aabbcc".to_string())
        );
    }

    #[test]
    fn decode_id_to_hex_lowercases_uppercase_hex() {
        assert_eq!(
            decode_id_to_hex(&json!("AABBCC")),
            Some("aabbcc".to_string())
        );
    }

    #[test]
    fn decode_id_to_hex_falls_back_to_base64() {
        // base64 of bytes [0xAA, 0xBB, 0xCC] is "qrvM"
        assert_eq!(decode_id_to_hex(&json!("qrvM")), Some("aabbcc".to_string()));
    }

    #[test]
    fn decode_id_to_hex_returns_none_for_empty_string() {
        assert_eq!(decode_id_to_hex(&json!("")), None);
    }

    #[test]
    fn parse_nanos_accepts_string_form() {
        assert_eq!(parse_nanos(&json!("1234567890")), Some(1_234_567_890));
    }

    #[test]
    fn parse_nanos_accepts_number_form() {
        assert_eq!(parse_nanos(&json!(1_234_567_890u64)), Some(1_234_567_890));
    }

    #[test]
    fn span_kind_label_handles_string_and_numeric_forms() {
        assert_eq!(span_kind_label(Some(&json!("SPAN_KIND_CLIENT"))), "CLIENT");
        assert_eq!(span_kind_label(Some(&json!(3))), "CLIENT");
        assert_eq!(span_kind_label(None), "UNSPECIFIED");
    }

    #[test]
    fn status_code_label_handles_string_and_numeric_forms() {
        assert_eq!(
            status_code_label(Some(&json!("STATUS_CODE_ERROR"))),
            "ERROR"
        );
        assert_eq!(status_code_label(Some(&json!(2))), "ERROR");
        assert_eq!(status_code_label(None), "UNSET");
    }

    #[test]
    fn format_any_value_renders_string_and_int() {
        assert_eq!(format_any_value(&json!({"stringValue": "hello"})), "hello");
        assert_eq!(format_any_value(&json!({"intValue": "200"})), "200");
    }

    #[test]
    fn format_any_value_renders_nested_array() {
        let v = json!({"arrayValue": {"values": [{"stringValue": "a"}, {"intValue": "1"}]}});
        assert_eq!(format_any_value(&v), "[a, 1]");
    }

    #[test]
    fn format_duration_nanos_scales_units_correctly() {
        assert_eq!(format_duration_nanos(500), "500ns");
        assert_eq!(format_duration_nanos(1_500), "1.50µs");
        assert_eq!(format_duration_nanos(2_500_000), "2.50ms");
        assert_eq!(format_duration_nanos(1_500_000_000), "1.500s");
    }

    #[test]
    fn flatten_extracts_service_name_from_resource_attributes() {
        let json = r#"{"resourceSpans":[{
            "resource": {"attributes": [{"key": "service.name", "value": {"stringValue": "checkout"}}]},
            "scopeSpans": [{"scope": {"name": "mytracer"}, "spans": [{
                "traceId": "aabb", "spanId": "1122", "name": "op",
                "startTimeUnixNano": "100", "endTimeUnixNano": "200"
            }]}]
        }]}"#;
        let doc = crate::otlp::parse(json).unwrap();
        let flat = flatten(&doc);
        assert_eq!(flat.len(), 1);
        assert_eq!(flat[0].service_name.as_deref(), Some("checkout"));
        assert_eq!(flat[0].scope_name, "mytracer");
        assert_eq!(flat[0].duration_nanos(), Some(100));
    }

    #[test]
    fn build_forest_nests_children_under_matching_parent_span_id() {
        let spans = vec![
            FlatSpan {
                service_name: None,
                scope_name: String::new(),
                trace_id: "t1".into(),
                span_id: "root".into(),
                parent_span_id: None,
                name: "root-op".into(),
                kind: "SERVER",
                start_nanos: 0,
                end_nanos: Some(100),
                attributes: vec![],
                status_code: "UNSET",
                status_message: None,
                events: vec![],
            },
            FlatSpan {
                service_name: None,
                scope_name: String::new(),
                trace_id: "t1".into(),
                span_id: "child".into(),
                parent_span_id: Some("root".into()),
                name: "child-op".into(),
                kind: "CLIENT",
                start_nanos: 10,
                end_nanos: Some(50),
                attributes: vec![],
                status_code: "UNSET",
                status_message: None,
                events: vec![],
            },
        ];
        let forest = build_forest(&spans);
        assert_eq!(forest.len(), 1);
        let (trace_id, roots) = &forest[0];
        assert_eq!(trace_id, "t1");
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].span.name, "root-op");
        assert_eq!(roots[0].children.len(), 1);
        assert_eq!(roots[0].children[0].span.name, "child-op");
    }

    #[test]
    fn build_forest_treats_span_with_missing_parent_as_root() {
        let spans = vec![FlatSpan {
            service_name: None,
            scope_name: String::new(),
            trace_id: "t1".into(),
            span_id: "orphan".into(),
            parent_span_id: Some("does-not-exist".into()),
            name: "orphan-op".into(),
            kind: "INTERNAL",
            start_nanos: 0,
            end_nanos: None,
            attributes: vec![],
            status_code: "UNSET",
            status_message: None,
            events: vec![],
        }];
        let forest = build_forest(&spans);
        assert_eq!(forest[0].1.len(), 1);
        assert_eq!(forest[0].1[0].span.name, "orphan-op");
    }

    #[test]
    fn build_forest_groups_multiple_traces_separately() {
        let make_span = |trace_id: &str, span_id: &str| FlatSpan {
            service_name: None,
            scope_name: String::new(),
            trace_id: trace_id.into(),
            span_id: span_id.into(),
            parent_span_id: None,
            name: "op".into(),
            kind: "INTERNAL",
            start_nanos: 0,
            end_nanos: None,
            attributes: vec![],
            status_code: "UNSET",
            status_message: None,
            events: vec![],
        };
        let spans = vec![make_span("t1", "a"), make_span("t2", "b")];
        let forest = build_forest(&spans);
        assert_eq!(forest.len(), 2);
    }

    #[test]
    fn render_trace_tree_shows_nesting_duration_and_status() {
        let json = r#"{"resourceSpans":[{
            "resource": {"attributes": [{"key": "service.name", "value": {"stringValue": "checkout-service"}}]},
            "scopeSpans": [{"scope": {"name": "otlpcat.livetest"}, "spans": [
                {
                    "traceId": "e8f9450dcbd4e6288fb17942f5cfa8c8",
                    "spanId": "859", "name": "handle-checkout", "kind": "SPAN_KIND_SERVER",
                    "startTimeUnixNano": "1000000000", "endTimeUnixNano": "1010000000"
                },
                {
                    "traceId": "e8f9450dcbd4e6288fb17942f5cfa8c8",
                    "spanId": "abc", "parentSpanId": "859", "name": "db-insert-order", "kind": "SPAN_KIND_CLIENT",
                    "startTimeUnixNano": "1005000000", "endTimeUnixNano": "1005140000",
                    "status": {"code": "STATUS_CODE_ERROR", "message": "connection timeout"}
                }
            ]}]
        }]}"#;
        let doc = crate::otlp::parse(json).unwrap();
        let flat = flatten(&doc);
        let rendered = render_trace_tree(&flat);

        assert!(rendered.contains("Trace e8f9450dcbd4e6288fb17942f5cfa8c8"));
        assert!(rendered.contains("handle-checkout [SERVER] service=checkout-service (10.00ms)"));
        assert!(rendered.contains("db-insert-order [CLIENT] service=checkout-service (140.00µs)"));
        assert!(rendered.contains("STATUS=ERROR: connection timeout"));
        // Child must be indented under the parent (nested block, not a second root).
        let parent_line = rendered
            .lines()
            .find(|l| l.contains("handle-checkout"))
            .unwrap();
        let child_line = rendered
            .lines()
            .find(|l| l.contains("db-insert-order"))
            .unwrap();
        assert!(!parent_line.starts_with(' '));
        assert!(
            child_line.starts_with("   ")
                || child_line.starts_with("├")
                || child_line.starts_with("└")
        );
    }

    #[test]
    fn render_trace_tree_marks_unfinished_spans_without_end_time() {
        let spans = vec![FlatSpan {
            service_name: None,
            scope_name: String::new(),
            trace_id: "t1".into(),
            span_id: "a".into(),
            parent_span_id: None,
            name: "still-running".into(),
            kind: "INTERNAL",
            start_nanos: 0,
            end_nanos: None,
            attributes: vec![],
            status_code: "UNSET",
            status_message: None,
            events: vec![],
        }];
        let rendered = render_trace_tree(&spans);
        assert!(rendered.contains("(unfinished)"));
    }
}
