use std::io::Read as _;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use otlpcat::otlp;
use otlpcat::render::{flatten, render_trace_tree};

#[derive(Parser)]
#[command(
    name = "otlpcat",
    about = "Pretty-prints OTLP/OpenTelemetry trace export JSON as a human-readable span tree"
)]
struct Cli {
    /// OTLP trace JSON file to read (an ExportTraceServiceRequest document). Reads stdin if omitted.
    file: Option<PathBuf>,
    /// Only show spans belonging to this trace id (hex).
    #[arg(long)]
    trace: Option<String>,
    /// Only show traces that contain at least one span with status ERROR.
    #[arg(long)]
    errors_only: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let input = match &cli.file {
        Some(path) => {
            std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))
        }
        None => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map(|_| buf)
                .map_err(|e| format!("reading stdin: {e}"))
        }
    };
    let input = match input {
        Ok(s) => s,
        Err(e) => {
            eprintln!("otlpcat: {e}");
            return ExitCode::FAILURE;
        }
    };

    let doc = match otlp::parse(&input) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("otlpcat: parsing OTLP JSON: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut spans = flatten(&doc);
    if spans.is_empty() {
        println!("otlpcat: no spans found in input");
        return ExitCode::SUCCESS;
    }

    if let Some(trace_id) = &cli.trace {
        let wanted = trace_id.to_lowercase();
        spans.retain(|s| s.trace_id == wanted);
        if spans.is_empty() {
            eprintln!("otlpcat: no spans found for trace id '{trace_id}'");
            return ExitCode::FAILURE;
        }
    }

    if cli.errors_only {
        let error_trace_ids: std::collections::HashSet<String> = spans
            .iter()
            .filter(|s| s.status_code == "ERROR")
            .map(|s| s.trace_id.clone())
            .collect();
        spans.retain(|s| error_trace_ids.contains(&s.trace_id));
        if spans.is_empty() {
            println!("otlpcat: no traces with an ERROR-status span found");
            return ExitCode::SUCCESS;
        }
    }

    print!("{}", render_trace_tree(&spans));
    ExitCode::SUCCESS
}
