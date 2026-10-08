//! `exec` primitive: inject a JSON array of editor messages and run them through the session.
//!
//! ```json
//! [{"Portfolio":{"Document":{"NodeGraph":{"SetInputValue":{"node_id":333,"input_index":2,"value":{"F64":42.5}}}}}},
//!  {"Portfolio":{"Document":{"NodeGraph":{"SelectedNodesSet":{"nodes":[333]}}}}}]
//! ```
//!
//! Message serialization is externally tagged down the full hierarchy
//! (`Message → Portfolio → Document → NodeGraph → …`). The measured shapes of the types agents
//! emit are documented in `cli/docs/message-shapes.md` and asserted by the integration test.
//!
//! Known limitations (CLI 方案 §四.5): messages with `serde(skip)` payload fields and
//! `ByteBuf`-style payloads cannot be expressed in JSON. They surface as a deserialization error
//! naming the offending array index — reported clearly, never a panic.

use crate::session::HeadlessSession;
use graphite_editor::messages::prelude::*;
use serde::Serialize;

/// Summary contract of the `exec` primitive.
#[derive(Debug, Clone, Serialize)]
pub struct ExecSummary {
	/// Every intercepted frontend side effect, in order (see `HeadlessSession::events`).
	pub events: Vec<serde_json::Value>,
	/// Number of failures: messages whose dispatch errored plus dialogs the editor raised.
	pub error_count: usize,
	/// Path written via `--save`, if requested.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub saved_path: Option<String>,
}

/// Parse the JSON array of editor messages without executing anything (schema smoke check).
///
/// The top-level `Message` enum derives serde, so every message is an externally tagged chain.
/// Malformed or JSON-inexpressible messages produce an error that names the array index and the
/// usual causes (`serde(skip)` fields like `UpdateTypes`, `ByteBuf` payloads).
pub fn parse_messages(input: &str) -> Result<Vec<Message>, String> {
	serde_json::from_str::<Vec<Message>>(input).map_err(|error| {
		format!(
			"Invalid message JSON array: {error}\n\
			 Note: messages whose payloads carry serde(skip) fields (e.g. UpdateTypes) or ByteBuf \
			 data cannot be constructed from JSON and are not supported by `exec`."
		)
	})
}

/// Execute a batch of messages one by one (each followed by a settle), then summarize the
/// intercepted side effects. When `save` is given, the document is finally saved through the
/// editor's full `.gdd` pipeline and written to that path.
///
/// The whole batch is wrapped in a document transaction (`StartTransaction` → messages →
/// `EndTransaction`): without it, message-level mutations (e.g. `SetInputValue`) only reach the
/// in-memory network interface while the graph-storage registry is snapshotted at transaction
/// commit — a bare batch then saves a drifted `.gdd` (legacy blob = new values, registry = old
/// values) that rolls back on reopen (measured). Read-only batches hit the `Started` branch and
/// cancel out harmlessly.
///
/// Per-message dispatch errors are recorded (error event + `error_count`) and do not abort the
/// batch; the command layer turns a nonzero `error_count` into a nonzero exit code.
pub async fn execute(session: &mut HeadlessSession, messages: Vec<Message>, save: Option<&std::path::Path>) -> Result<ExecSummary, String> {
	session.clear_tracking();
	let mut error_count = 0usize;

	session.dispatch(DocumentMessage::StartTransaction);
	for (index, message) in messages.into_iter().enumerate() {
		if let Err(error) = session.execute(message).await {
			error_count += 1;
			session.events.push(serde_json::json!({
				"kind": "error",
				"message_index": index,
				"detail": error,
			}));
		}
	}
	session.dispatch(DocumentMessage::EndTransaction);
	session.settle().await?;

	let mut saved_path = None;
	if let Some(path) = save {
		match save_active_document(session, path).await {
			Ok(written) => saved_path = Some(written.display().to_string()),
			Err(error) => {
				error_count += 1;
				session.events.push(serde_json::json!({ "kind": "error", "detail": error }));
			}
		}
	}

	error_count += session.dialogs_shown.len();

	Ok(ExecSummary {
		events: std::mem::take(&mut session.events),
		error_count,
		saved_path,
	})
}

/// Save the active document as `.gdd` through the editor pipeline and write it to `path`.
/// Same chain as `save_active_document_as_gdd`, but tolerant of a missing capture (reported as
/// an error event instead of failing the whole batch).
async fn save_active_document(session: &mut HeadlessSession, path: &std::path::Path) -> Result<std::path::PathBuf, String> {
	let outcome = session.save_active_document_as_gdd(Some(path.to_path_buf())).await?;
	Ok(outcome.path)
}

/// Render the summary as one JSON line on stdout (the `exec` stdout contract).
pub fn print_summary(summary: &ExecSummary) {
	let line = serde_json::json!({
		"events": summary.events,
		"error_count": summary.error_count,
		"saved_path": summary.saved_path,
	});
	println!("{line}");
}
