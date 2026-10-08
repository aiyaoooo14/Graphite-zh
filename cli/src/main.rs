//! `graphite-cli` — headless, message-level Graphite editor CLI.
//!
//! Unlike `graphene-cli` (which drives the pure compiler stack), this tool runs the **full
//! editor** and manipulates documents through the editor message channel. See
//! `document/zh/CLI接口方案.md` §三 for the architecture contract.

use clap::{Parser, Subcommand};
use graphite_cli::session::HeadlessSession;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "graphite-cli", version, about = "Headless Graphite editor CLI (message-level)")]
struct Cli {
	#[command(subcommand)]
	command: Command,

	/// Directory for the editor's mmap-backed resource storage.
	#[arg(long, default_value_os_t = default_resource_cache())]
	resource_cache: PathBuf,

	/// Log filter in env_logger syntax (e.g. "info", "graphite_editor=debug"); RUST_LOG overrides.
	#[arg(long, default_value = "warn")]
	log_filter: String,
}

fn default_resource_cache() -> PathBuf {
	std::env::temp_dir().join("graphite-cli-resources")
}

#[derive(Debug, Subcommand)]
enum Command {
	/// Open a document, drive the editor to quiescence, and print a node-graph summary.
	Eval {
		/// Path to a .graphite (legacy) or .gdd document.
		document: PathBuf,
	},
	/// Open a document and save it as a .gdd container via the editor's save pipeline.
	Save {
		/// Path to the input document (.graphite or .gdd).
		document: PathBuf,
		/// Output path (defaults to the input path with a .gdd extension).
		#[arg(short, long)]
		output: Option<PathBuf>,
	},
	/// Open a document and print its structure (nodes, wires, layer tree) as JSON.
	Inspect {
		/// Path to a .graphite (legacy) or .gdd document.
		document: PathBuf,
		/// Pretty-print the JSON for human reading.
		#[arg(long)]
		pretty: bool,
	},
	/// Inject a JSON array of editor messages, run them, and print an event summary.
	///
	/// Exactly one message source must be given: `--json-string <s>` or `--file <f>`.
	Exec {
		/// Path to a .graphite (legacy) or .gdd document to open first.
		document: PathBuf,
		/// Inline JSON array of editor messages, e.g.
		/// `--json-string '[{"Portfolio":{"Document":{"NodeGraph":{"SelectedNodesSet":{"nodes":[1]}}}}}]'`.
		#[arg(long, conflicts_with = "file")]
		json_string: Option<String>,
		/// JSON file containing a `[{…message…}, …]` array.
		#[arg(long, conflicts_with = "json_string")]
		file: Option<PathBuf>,
		/// Save the document to this path after the batch (as .gdd).
		#[arg(long)]
		save: Option<PathBuf>,
	},
	/// Export the document through the editor chain (SubmitDocumentExport → encoded file).
	Export {
		/// Path to a .graphite (legacy) or .gdd document.
		document: PathBuf,
		/// Output path; the extension selects the format (.svg or .png).
		#[arg(short, long)]
		output: PathBuf,
		/// Export width in pixels (overrides --scale via a corrective pass).
		#[arg(long)]
		width: Option<u32>,
		/// Export height in pixels (overrides --scale via a corrective pass).
		#[arg(long)]
		height: Option<u32>,
		/// Scale factor applied to the artwork bounds (default 1.0).
		#[arg(long, default_value_t = 1.0)]
		scale: f64,
	/// Accepted for compatibility; PNG exports are always RGBA (transparent background).
	#[arg(long)]
	transparent: bool,
	},
}

fn main() -> std::process::ExitCode {
	let cli = Cli::parse();

	let _ = env_logger::Builder::new()
		.parse_filters(&cli.log_filter)
		.parse_default_env()
		.try_init();

	// Hand-rolled current-thread runtime instead of `#[tokio::main]`: the session (and the async
	// spawner runtime embedded in the editor) must be dropped *outside* of an async context, and
	// tokio forbids dropping a runtime from within one. `#[tokio::main]` would run this whole
	// function inside `block_on`, so the session is parked in a slot that is dropped here, after
	// the async context has ended.
	let runtime = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.expect("failed to build tokio current-thread runtime");

	let mut session_slot: Option<HeadlessSession> = None;
	let result = runtime.block_on(run(cli, &mut session_slot));

	drop(session_slot);
	drop(runtime);

	match result {
		Ok(()) => std::process::ExitCode::SUCCESS,
		Err(error) => {
			eprintln!("error: {error}");
			std::process::ExitCode::FAILURE
		}
	}
}

async fn run(cli: Cli, session_slot: &mut Option<HeadlessSession>) -> Result<(), String> {
	// One editor per process (ENVIRONMENT is a OnceLock); the session must also stay on this
	// thread (the editor is not Send due to wgpu), hence the current-thread runtime. The session
	// lives in `session_slot` so the caller can drop it outside of the async context.
	session_slot.replace(HeadlessSession::new(&cli.resource_cache).await?);
	let session = session_slot.as_mut().expect("session just inserted");

	use graphite_cli::exec;
	use graphite_cli::export;
	use graphite_cli::inspect;

	match cli.command {
		Command::Eval { document } => {
			session.open_document(&document).await?;
			session.settle().await?;
			println!("{}: evaluated to quiescence ({} suppressed frontend messages)", document.display(), session.suppressed_messages.len());
			if !session.dialogs_shown.is_empty() {
				return Err("Editor raised dialogs during evaluation".to_string());
			}
		}
		Command::Save { document, output } => {
			session.open_document(&document).await?;
			let outcome = session.save_active_document_as_gdd(output).await?;
			println!("Saved {} -> {} ({} bytes)", document.display(), outcome.path.display(), outcome.bytes);
		}
		Command::Inspect { document, pretty } => {
			// Probe first: compiler-stack `.gdd` files (no embedded editor document) would panic
			// inside the editor's graph flattening, so they are routed to the registry read path
			// before any editor interaction. Editor-backed documents go through the editor's own
			// view of the document (captured via the save pipeline — never frontend messages).
			let is_gdd = document.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("gdd"));
			let inspected = if is_gdd && inspect::probe_gdd_document(&document).await? == inspect::DocumentProbe::RegistryOnly {
				eprintln!("note: {} carries no embedded editor document; inspecting the compiler-stack registry instead", document.display());
				let network = inspect::read_registry_network(&document).await?;
				inspect::inspect_registry_network(&network)
			} else {
				session.open_document(&document).await?;
				let snapshot = session.capture_document_json().await?;
				inspect::inspect_editor_snapshot(&snapshot)?
			};
			let text = if pretty {
				serde_json::to_string_pretty(&inspected).expect("inspection serializes")
			} else {
				serde_json::to_string(&inspected).expect("inspection serializes")
			};
			println!("{text}");
		}
		Command::Exec { document, json_string, file, save } => {
			require_editor_backed(&document).await?;
			let input = match (json_string, file) {
				(Some(json), None) => json,
				(None, Some(path)) => {
					std::fs::read_to_string(&path).map_err(|error| format!("Failed to read {}: {error}", path.display()))?
				}
				(None, None) => return Err("exec requires exactly one message source: --json-string <s> or --file <f>".to_string()),
				// clap's `conflicts_with` enforces exclusivity before we ever get here.
				(Some(_), Some(_)) => return Err("exec accepts only one message source".to_string()),
			};
			session.open_document(&document).await?;
			let messages = exec::parse_messages(&input)?;
			let summary = exec::execute(session, messages, save.as_deref()).await?;
			exec::print_summary(&summary);
			if summary.error_count > 0 {
				return Err(format!("exec finished with {} error(s)", summary.error_count));
			}
		}
		Command::Export { document, output, width, height, scale, transparent } => {
			require_editor_backed(&document).await?;
			let summary = export::run(session, &document, &output, width, height, scale, transparent).await?;
			export::print_summary(&summary);
		}
	}
	Ok(())
}

/// Reject compiler-stack `.gdd` files up front for editor-bound commands (`exec`/`export`): the
/// editor cannot open them and would panic inside graph flattening, so they get a clean error
/// instead (probe is a pure document-format read).
async fn require_editor_backed(document: &Path) -> Result<(), String> {
	let is_gdd = document.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("gdd"));
	if is_gdd && graphite_cli::inspect::probe_gdd_document(document).await? == graphite_cli::inspect::DocumentProbe::RegistryOnly {
		return Err(format!(
			"{} is a compiler-stack .gdd (no embedded editor document): the editor cannot open it, so `exec`/`export` cannot process it. \
			 Use `inspect` (registry read path) to view its structure.",
			document.display()
		));
	}
	Ok(())
}
