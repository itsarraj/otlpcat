//! Typed (but deliberately lenient) mirror of the OTLP trace export JSON
//! schema. "Lenient" because live testing against the real
//! `opentelemetry-python` SDK's own protobuf-JSON encoder showed real
//! producers disagree on two points the OTLP spec leaves as valid
//! alternate encodings: trace/span IDs can come out hex *or*
//! base64-of-the-raw-bytes, and enums (`kind`, `status.code`) can come
//! out as their string name *or* their numeric code. Fields that vary
//! this way are kept as raw `serde_json::Value` here and normalized by
//! the pure functions in `render.rs`, rather than assumed to be one
//! fixed shape.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ExportTraceServiceRequest {
    #[serde(rename = "resourceSpans", default)]
    pub resource_spans: Vec<ResourceSpans>,
}

#[derive(Debug, Deserialize)]
pub struct ResourceSpans {
    #[serde(default)]
    pub resource: Option<Resource>,
    #[serde(rename = "scopeSpans", default)]
    pub scope_spans: Vec<ScopeSpans>,
}

#[derive(Debug, Deserialize)]
pub struct Resource {
    #[serde(default)]
    pub attributes: Vec<KeyValue>,
}

#[derive(Debug, Deserialize)]
pub struct ScopeSpans {
    #[serde(default)]
    pub scope: Option<Scope>,
    #[serde(default)]
    pub spans: Vec<Span>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Scope {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct KeyValue {
    pub key: String,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct Span {
    #[serde(rename = "traceId")]
    pub trace_id: serde_json::Value,
    #[serde(rename = "spanId")]
    pub span_id: serde_json::Value,
    #[serde(rename = "parentSpanId", default)]
    pub parent_span_id: Option<serde_json::Value>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: Option<serde_json::Value>,
    #[serde(rename = "startTimeUnixNano")]
    pub start_time_unix_nano: serde_json::Value,
    #[serde(rename = "endTimeUnixNano", default)]
    pub end_time_unix_nano: Option<serde_json::Value>,
    #[serde(default)]
    pub attributes: Vec<KeyValue>,
    #[serde(default)]
    pub status: Option<StatusJson>,
    #[serde(default)]
    pub events: Vec<EventJson>,
}

#[derive(Debug, Deserialize)]
pub struct StatusJson {
    #[serde(default)]
    pub code: Option<serde_json::Value>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EventJson {
    #[serde(rename = "timeUnixNano", default)]
    pub time_unix_nano: Option<serde_json::Value>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub attributes: Vec<KeyValue>,
}

pub fn parse(json: &str) -> serde_json::Result<ExportTraceServiceRequest> {
    serde_json::from_str(json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_valid_document() {
        let doc = parse(r#"{"resourceSpans":[]}"#).unwrap();
        assert!(doc.resource_spans.is_empty());
    }

    #[test]
    fn parses_a_span_with_hex_ids_and_numeric_kind() {
        let json = r#"{
            "resourceSpans": [{
                "scopeSpans": [{
                    "spans": [{
                        "traceId": "e8f9450dcbd4e6288fb17942f5cfa8c8",
                        "spanId": "bfad9abd88535075",
                        "name": "test-span",
                        "kind": 3,
                        "startTimeUnixNano": "1000000000",
                        "endTimeUnixNano": "1000000500"
                    }]
                }]
            }]
        }"#;
        let doc = parse(json).unwrap();
        let span = &doc.resource_spans[0].scope_spans[0].spans[0];
        assert_eq!(
            span.trace_id,
            serde_json::Value::String("e8f9450dcbd4e6288fb17942f5cfa8c8".into())
        );
        assert_eq!(span.kind, Some(serde_json::Value::Number(3.into())));
    }

    #[test]
    fn missing_optional_fields_default_instead_of_erroring() {
        let json = r#"{
            "resourceSpans": [{
                "scopeSpans": [{
                    "spans": [{
                        "traceId": "aa",
                        "spanId": "bb",
                        "startTimeUnixNano": "1"
                    }]
                }]
            }]
        }"#;
        let doc = parse(json).unwrap();
        let span = &doc.resource_spans[0].scope_spans[0].spans[0];
        assert!(span.parent_span_id.is_none());
        assert!(span.status.is_none());
        assert!(span.events.is_empty());
        assert!(span.attributes.is_empty());
    }

    #[test]
    fn rejects_completely_invalid_json() {
        assert!(parse("not json at all").is_err());
    }
}
