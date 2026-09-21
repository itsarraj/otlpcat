# otlpcat

Pretty-prints OTLP/OpenTelemetry trace export JSON as a human-readable
span tree, instead of the deeply nested `resourceSpans → scopeSpans →
spans` JSON you get from a collector's file exporter, a captured
`curl -d @trace.json` payload, or a debug dump.

## Usage

```bash
otlpcat trace.json
otlpcat --trace 7bc7e5031bd4e6288fb17942f5cfa8c8 trace.json
otlpcat --errors-only trace.json
cat trace.json | otlpcat
```

## How it works

Parses an OTLP `ExportTraceServiceRequest` JSON document, flattens the
nested resource/scope/span structure into one list with resource and
scope context attached to each span, links each span under its parent
by `parentSpanId` within its trace (a span whose parent isn't present
in the document — normal for a single-service capture of a
distributed trace — becomes a root instead of an error), and renders
one `tree`-style indented block per trace: span name, kind, service,
duration, status (with message, for errors), attributes, and events
with their time offset from the span's start.

## A real, live-discovered format ambiguity this tool handles

While building this, I generated a genuine OTLP export using the
actual `opentelemetry-sdk` + `opentelemetry-exporter-otlp-proto-common`
Python packages and converted the real protobuf `ExportTraceServiceRequest`
to JSON with `google.protobuf.json_format.MessageToJson` — the same
class hierarchy real OTLP/HTTP exporters build on. Two things came out
differently than I'd first assumed, both confirmed live rather than
guessed:

1. **Trace/span IDs came out as standard base64**, not hex
   (`"e8flAxvU5iiPsXlC9c+oyA=="` rather than
   `"7bc7e5031bd4e6288fb17942f5cfa8c8"`). The OTLP spec's canonical
   JSON convention for these `bytes` fields is hex, but a generic
   protobuf-JSON marshaler has no OTLP-specific override for it, so
   different real encoders genuinely disagree. `otlpcat` detects which
   form it's looking at (valid-hex-and-even-length vs. not) and always
   *displays* hex, decoding base64 first when that's what it finds.
2. **`kind` and `status.code` came out as their string enum name**
   (`"SPAN_KIND_CLIENT"`, `"STATUS_CODE_ERROR"`) rather than a bare
   integer. Both forms are valid per the protobuf JSON mapping, and
   `otlpcat` accepts either.

## Status: built, unit-tested, and live-verified against a genuine OpenTelemetry SDK trace export, independently cross-checked

- **21 unit tests** (`cargo test --lib`) cover OTLP JSON parsing
  (minimal documents, hex ids with numeric `kind`, missing-optional-field
  defaulting, invalid-JSON rejection), the base64/hex id normalization
  described above, nanosecond timestamp parsing in both string and
  number form, `kind`/`status.code` label resolution in both string and
  numeric form, `AnyValue` rendering (string/int/nested array), duration
  formatting across all four unit scales, span flattening with resource
  attribute extraction, and forest-building (parent/child nesting,
  missing-parent-becomes-root, multiple traces kept separate). **Two
  real bugs caught by the tests themselves**, both wrong hand-computed
  expected values in the test, not the formatting code: a
  `10_000_000ns` duration test asserted `"10.000ms"` (three decimals,
  copying the seconds-branch format) when the millisecond branch
  actually — correctly — uses two decimals, `"10.00ms"`; and a stray
  string-replace hack in an assertion that should have just been the
  literal expected substring.
- **Live-verified against a real trace captured from the actual
  `opentelemetry-sdk` Python package** (not a hand-written fixture): a
  3-span trace (`handle-checkout` [SERVER] → `charge-card` [CLIENT],
  `db-insert-order` [CLIENT] with a real recorded exception event and
  `STATUS_CODE_ERROR`), exported through the SDK's real span processor
  and encoded via the real `opentelemetry-exporter-otlp-proto-common`
  protobuf encoder. `otlpcat` correctly rendered the parent/child tree,
  computed real millisecond/microsecond durations from the real
  nanosecond timestamps, and surfaced the error status and its message.
  **The base64→hex trace-id decode was independently cross-checked**:
  `python3 -c "base64.b64decode(...).hex()"` on the same raw id string
  produced `7bc7e5031bd4e6288fb17942f5cfa8c8`, byte-for-byte identical
  to what `otlpcat` printed — confirmed by a second, independent
  decoder, not just this tool's own claim about itself.
  Also verified in the same run: `--trace <id>` filtering to a single
  trace, `--trace` on a nonexistent id failing with a clear message and
  exit code 1, `--errors-only` correctly keeping the trace with the
  ERROR span, stdin-piped input producing identical output to file
  input, and malformed JSON input failing cleanly with exit code 1
  instead of panicking.

**Not done / deliberately deferred**: no OTLP/gRPC (protobuf wire
format) support, JSON only — matches what a collector's file/logging
exporter or a captured HTTP request body actually gives you; no logs
or metrics OTLP payloads, traces only; a multi-line attribute value
(e.g. a captured exception stacktrace with embedded newlines, seen
in the live test above) prints its embedded newlines as-is rather than
being escaped/indented, which is readable but not perfectly aligned in
the tree output.
