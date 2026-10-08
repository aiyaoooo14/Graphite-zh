//! Headless editor session.
//!
//! Builds the editor exactly like the desktop wrapper does (`desktop/wrapper/src/lib.rs:34-47`)
//! and drives it synchronously to quiescence, so commands like "open → save" work without a
//! UI event loop.
//!
//! Key design points:
//! - The editor is **not `Send`** (wgpu constraint) → the session must stay on one thread.
//!   The CLI therefore runs on a tokio `current_thread` runtime.
//! - `SaveDocument`/`OpenGddDocument` resolve through async futures whose results are only
//!   drained at the *start of the next* `handle_message` (`editor/src/dispatcher.rs:130-135`).
//!   Since our `Wake` closure is not tied to an event loop, [`HeadlessSession::settle`] waits
//!   on a `tokio::sync::Notify` that the wake closure signals, then keeps dispatching until
//!   nothing is left. Missing this loop means saves are silently lost (CLI 方案 §四.1).
//! - Frontend side effects are intercepted: `TriggerSaveDocument` is captured for the caller,
//!   dialogs/persistence/clipboard etc. are swallowed and logged.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use graph_craft::application_io::resource::MmapResourceStorage;
use graph_craft::application_io::resource::ResourceStorage;
use graph_craft::application_io::{PlatformApplicationIo, wgpu_available};
use graphite_editor::application::{Editor, Environment, Host, Platform};
use graphite_editor::messages::prelude::*;

/// Upper bound on driver rounds before we declare a (likely buggy) livelock.
const MAX_QUIESCENCE_ROUNDS: usize = 10_000;

/// When nothing reports activity and no async future completes within this window, the session
/// is considered quiescent. The wake closure fires the instant a spawned future resolves, so
/// under normal operation this timeout never delays exit; it only bounds a missed wake or a
/// future that never completes.
const QUIESCENCE_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

/// A document save captured from `FrontendMessage::TriggerSaveDocument`.
pub struct CapturedSave {
	pub document_id: DocumentId,
	pub name: String,
	pub path: Option<PathBuf>,
	pub folder: Option<PathBuf>,
	pub content: Vec<u8>,
}

/// A generic file payload captured from `FrontendMessage::TriggerSaveFile` (the editor's export
/// pipeline hands over fully encoded SVG/PNG bytes through this message).
pub struct CapturedFile {
	pub name: String,
	pub content: Vec<u8>,
}

/// A rasterization request captured from `FrontendMessage::TriggerExportImage` (SVG content the
/// frontend is expected to rasterize). In a headless session there is no such frontend, so the
/// export command treats this as an error signal rather than a deliverable.
pub struct CapturedExportImage {
	pub svg: String,
	pub name: String,
	pub mime: String,
	pub size: (f64, f64),
}

/// Result of a successful [`HeadlessSession::save_active_document_as_gdd`].
pub struct SaveOutcome {
	/// Path the document bytes were written to.
	pub path: PathBuf,
	/// Number of bytes written.
	pub bytes: usize,
}

pub struct HeadlessSession {
	editor: Editor,
	async_wake: Arc<tokio::sync::Notify>,
	saves: Vec<CapturedSave>,
	files: Vec<CapturedFile>,
	export_images: Vec<CapturedExportImage>,
	/// Titles of dialogs the editor tried to show (a failed open shows one).
	pub dialogs_shown: Vec<String>,
	/// Debug representations of frontend messages swallowed by the interceptor.
	pub suppressed_messages: Vec<String>,
	/// Machine-readable record of every intercepted frontend side effect, in order
	/// (`exec`/`export` summaries surface this as their `events` array).
	pub events: Vec<serde_json::Value>,
}

impl HeadlessSession {
	/// Build the editor and initialize the portfolio.
	///
	/// Note: the editor `ENVIRONMENT` is a process-wide `OnceLock` (`editor/src/application.rs:21`),
	/// so only one session can exist per process. Integration tests must run with `--test-threads=1`.
	pub async fn new(resource_storage_root: impl Into<PathBuf>) -> Result<Self, String> {
		let resource_storage: Arc<dyn ResourceStorage> = Arc::from(Box::new(
			MmapResourceStorage::new(resource_storage_root).map_err(|error| format!("Failed to set up resource storage: {error}"))?,
		) as Box<dyn ResourceStorage>);

		log::info!("Initializing headless wgpu context");
		let application_io = PlatformApplicationIo::new().await;
		if wgpu_available() == Some(false) {
			return Err("No wgpu/GPU backend available; the headless editor session requires wgpu (CLI 方案 §四.8)".to_string());
		}

		// Fixed UUID seed: the same operation sequence yields the same NodeIds. This
		// determinism is a feature — agent tooling keys off stable NodeIds (CLI 方案 §一.4).
		const UUID_RANDOM_SEED: u64 = 0x5EED_2026_1008;

		let async_wake = Arc::new(tokio::sync::Notify::new());
		let wake: Wake = {
			let notify = async_wake.clone();
			Arc::new(move || notify.notify_one())
		};

		let editor = Editor::new(
			Environment { platform: Platform::Desktop, host: Host::Linux },
			UUID_RANDOM_SEED,
			resource_storage,
			// In-memory working copy: no stray files outside the resource storage.
			None,
			application_io,
			wake,
		);

		let mut session = Self {
			editor,
			async_wake,
			saves: Vec::new(),
			files: Vec::new(),
			export_images: Vec::new(),
			dialogs_shown: Vec::new(),
			suppressed_messages: Vec::new(),
			events: Vec::new(),
		};
		session.dispatch(PortfolioMessage::Init);
		Ok(session)
	}

	/// Open a document through the portfolio message layer. The editor dispatches by file
	/// extension: `.gdd` → `OpenGddDocument`, legacy `.graphite` → `OpenDocumentFile`
	/// (`PortfolioMessage::OpenFile` → `read_file`, `portfolio_message_handler.rs:1802`).
	pub async fn open_document(&mut self, path: &Path) -> Result<(), String> {
		let content = std::fs::read(path).map_err(|error| format!("Failed to read document {}: {error}", path.display()))?;
		let dialogs_before = self.dialogs_shown.len();

		self.dispatch(PortfolioMessage::OpenFile { path: path.to_path_buf(), content });
		self.settle().await?;

		if self.dialogs_shown.len() > dialogs_before {
			return Err(format!("Editor reported an error dialog while opening {}", path.display()));
		}
		Ok(())
	}

	/// Save the active document as a `.gdd` container through the editor's full save pipeline
	/// (resource GC → embed → working-copy export), write the captured bytes to disk, and close
	/// the loop with `DocumentMessage::SavedDocument` (as the desktop does in
	/// `handle_desktop_wrapper_message.rs:28-31`).
	///
	/// `output` overrides the path the editor suggests (input path with a `.gdd` extension).
	pub async fn save_active_document_as_gdd(&mut self, output: Option<PathBuf>) -> Result<SaveOutcome, String> {
		self.saves.clear();

		// `save_as_gdd` defaults to `false`: without this the save falls back to legacy
		// `.graphite` (preferences_message_handler.rs:73, CLI 方案 §四.2).
		self.dispatch(PreferencesMessage::SaveAsGdd { enabled: true });
		self.dispatch(PortfolioMessage::Document(DocumentMessage::SaveDocument));
		self.settle().await?;

		let mut saves = std::mem::take(&mut self.saves);
		let captured = saves
			.pop()
			.ok_or_else(|| "Save produced no TriggerSaveDocument — the save pipeline never reached quiescence".to_string())?;
		if !saves.is_empty() {
			log::warn!("Save produced {} extra TriggerSaveDocument messages beyond the expected one", saves.len());
		}

		let path = match output {
			Some(output) => output,
			None => captured
				.path
				.clone()
				.ok_or_else(|| "Editor suggested no save path and no --output was given".to_string())?,
		};

		if let Some(parent) = path.parent()
			&& !parent.as_os_str().is_empty()
		{
			std::fs::create_dir_all(parent).map_err(|error| format!("Failed to create directory {}: {error}", parent.display()))?;
		}
		std::fs::write(&path, &captured.content).map_err(|error| format!("Failed to write {}: {error}", path.display()))?;

		// Report the actual written path back so the editor's document name/path state stays in
		// sync (document_message_handler.rs:1093-1111, CLI 方案 §四.3).
		self.dispatch(PortfolioMessage::DocumentPassMessage {
			document_id: captured.document_id,
			message: DocumentMessage::SavedDocument { path: Some(path.clone()) },
		});
		self.settle().await?;

		Ok(SaveOutcome { path, bytes: captured.content.len() })
	}

	/// Capture the current document as serialized JSON through the editor's own save pipeline
	/// (resource GC → embed → `serialize_document`), **without writing to disk**. This is the
	/// same authoritative snapshot a legacy save would produce, so `inspect` can read document
	/// structure straight from the editor's document format instead of waiting for frontend
	/// messages (which are gated on UI state like `graph_view_overlay_open`).
	pub async fn capture_document_json(&mut self) -> Result<Vec<u8>, String> {
		self.saves.clear();
		self.dispatch(PreferencesMessage::SaveAsGdd { enabled: false }); // plain legacy JSON blob
		self.dispatch(PortfolioMessage::Document(DocumentMessage::SaveDocument));
		self.settle().await?;

		let mut saves = std::mem::take(&mut self.saves);
		let captured = saves
			.pop()
			.ok_or_else(|| "capture_document_json produced no TriggerSaveDocument — the save pipeline never reached quiescence".to_string())?;
		Ok(captured.content)
	}

	/// Dispatch a single message and drive the session back to quiescence.
	pub async fn execute(&mut self, message: impl Into<Message>) -> Result<(), String> {
		self.dispatch(message);
		self.settle().await
	}

	/// Number of `TriggerSaveDocument` captures intercepted so far (since the last clear).
	pub fn save_count(&self) -> usize {
		self.saves.len()
	}

	/// Take every captured `TriggerSaveDocument` (draining them), in order.
	pub fn take_captured_saves(&mut self) -> Vec<CapturedSave> {
		std::mem::take(&mut self.saves)
	}

	/// Take every captured `TriggerSaveFile` payload (draining them), in order. These carry the
	/// fully encoded export artifacts (SVG text / PNG bytes) produced by the editor's export chain.
	pub fn take_captured_files(&mut self) -> Vec<CapturedFile> {
		std::mem::take(&mut self.files)
	}

	/// Take every captured `TriggerExportImage` requests (draining them), in order.
	pub fn take_captured_export_images(&mut self) -> Vec<CapturedExportImage> {
		std::mem::take(&mut self.export_images)
	}

	/// Reset every interceptor-side tracker (captures, dialogs, suppressed list, events). The
	/// `exec`/`export` commands call this so their summaries only cover their own batch.
	pub fn clear_tracking(&mut self) {
		self.saves.clear();
		self.files.clear();
		self.export_images.clear();
		self.dialogs_shown.clear();
		self.suppressed_messages.clear();
		self.events.clear();
	}

	/// Drive the editor until quiescence: no pending messages, no node graph work, and no async
	/// future results in flight. Mirrors the desktop app loop (dispatch → `run_node_graph` →
	/// `poll_node_graph_evaluation` → dispatch polled messages) as a closed loop instead of an
	/// event-driven one.
	pub async fn settle(&mut self) -> Result<(), String> {
		let mut pending: VecDeque<Message> = VecDeque::new();
		for _round in 0..MAX_QUIESCENCE_ROUNDS {
			let mut activity = false;

			// 1) Dispatch every pending message (each call also drains async results produced
			//    by earlier rounds at the top of `handle_message`).
			while let Some(message) = pending.pop_front() {
				activity = true;
				self.dispatch(message);
			}

			// 2) Drive the node graph runtime until the current submission completes. Note the
			//    boolean result means "runtime present and lock acquired", not "did work", so it
			//    must not feed the activity check (this is what the desktop only uses as a
			//    completion signal before polling).
			graphite_editor::node_graph_executor::run_node_graph().await;

			// 3) Turn the finished evaluation into messages for the editor to consume.
			let mut polled: VecDeque<Message> = VecDeque::new();
			if let Err(error) = self.editor.poll_node_graph_evaluation(&mut polled) {
				if error != "No active document" {
					return Err(format!("Node graph evaluation failed: {error}"));
				}
			}
			if !polled.is_empty() {
				activity = true;
				pending.extend(polled);
			}

			// 4) Tick the future channel: this dispatch's head-of-tick drain collects any async
			//    results that completed so far (same as the desktop's `FutureMessage::Wake`).
			self.dispatch(FutureMessage::Wake);

			if !activity {
				// Async message futures (e.g. the save pipeline) run on the spawner thread and
				// call our wake closure the moment they resolve, so this await returns promptly
				// while work is outstanding and only times out when truly quiescent.
				if tokio::time::timeout(QUIESCENCE_IDLE_TIMEOUT, self.async_wake.notified()).await.is_err() {
					return Ok(());
				}
			}
		}
		Err("Session failed to reach quiescence within the round limit".to_string())
	}

	/// Dispatch a message to the editor and intercept its frontend side effects.
	pub fn dispatch(&mut self, message: impl Into<Message>) {
		let frontend_messages = self.editor.handle_message(message);
		for message in frontend_messages {
			self.intercept_frontend_message(message);
		}
	}

	/// Frontend side-effect interception, modeled after the desktop wrapper's
	/// `intercept_frontend_message.rs` but with no real "frontend" behind it.
	pub(crate) fn intercept_frontend_message(&mut self, message: FrontendMessage) {
		match message {
			// The actual save payload: content is written to disk by the command layer.
			FrontendMessage::TriggerSaveDocument { document_id, name, path, folder, content } => {
				self.events.push(serde_json::json!({
					"kind": "save_document",
					"name": name,
					"path": path.as_ref().map(|path| path.display().to_string()),
					"bytes": content.len(),
				}));
				self.saves.push(CapturedSave {
					document_id,
					name,
					path,
					folder,
					content: content.into_vec(),
				});
			}
			// A fully encoded file the editor wants the frontend to save (export pipeline output).
			FrontendMessage::TriggerSaveFile { name, folder: _, content } => {
				self.events.push(serde_json::json!({
					"kind": "save_file",
					"name": name,
					"bytes": content.len(),
				}));
				self.files.push(CapturedFile { name, content: content.into_vec() });
			}
			// An SVG the editor wants the frontend to rasterize: in headless mode nobody can
			// fulfill this, so it is captured for the export command to surface as an error.
			FrontendMessage::TriggerExportImage { svg, name, mime, size } => {
				self.events.push(serde_json::json!({
					"kind": "export_image",
					"name": name,
					"mime": mime,
					"size": [size.0, size.1],
				}));
				self.export_images.push(CapturedExportImage { svg, name, mime, size });
			}
			// A dialog means something went wrong (e.g. failed open) — surface it.
			FrontendMessage::DisplayDialog { title, .. } => {
				log::error!("Editor requested a dialog: {title}");
				self.events.push(serde_json::json!({ "kind": "dialog", "title": title }));
				self.dialogs_shown.push(title);
			}
			// Preferences and persistence stay in-process (defaults + explicit messages only).
			FrontendMessage::TriggerSavePreferences { .. }
			| FrontendMessage::TriggerLoadPreferences
			| FrontendMessage::TriggerPersistenceReadState
			| FrontendMessage::TriggerPersistenceWriteState { .. }
			| FrontendMessage::TriggerPersistenceReadDocument { .. }
			| FrontendMessage::TriggerPersistenceWriteDocument { .. }
			| FrontendMessage::TriggerPersistenceDeleteDocument { .. }
			| FrontendMessage::TriggerOpenLaunchDocuments => {
				self.suppressed_messages.push("Trigger* (preferences/persistence)".to_string());
				self.events.push(serde_json::json!({ "kind": "suppressed", "message": "preferences/persistence" }));
				log::debug!("Suppressed frontend message (headless)");
			}
			// Everything else (viewport updates, layout, clipboard, window controls, …) is
			// meaningless without a UI.
			other => {
				self.suppressed_messages.push("frontend update".to_string());
				self.events.push(serde_json::json!({ "kind": "suppressed", "message": "frontend-update" }));
				log::trace!("Ignoring frontend message (headless): {other:?}");
			}
		}
	}
}
