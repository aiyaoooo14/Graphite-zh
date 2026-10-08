//! Round-trip hard gate (CLI 方案 §四.10): save → reopen → assert node/resource consistency.
//!
//! Covers both required directions in one flow:
//!   Phase 1: legacy `.graphite` → save as `.gdd`
//!   Phase 2: reopen `.gdd` → save as `.gdd` → node/resource equality
//!   Phase 3: reopen again → save again → still equal (resave stability)
//!
//! Everything lives in a single test because the editor `ENVIRONMENT` is a process-wide
//! `OnceLock` (`editor/src/application.rs:21`): only one session can exist per process.
//! **MUST** be run with `--test-threads=1`.

use document_container::AnyContainer;
use document_container::backends::memory::MemoryBackend;
use document_format::{Gdd, GddV1Layout};
use graphite_cli::session::HeadlessSession;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Fails the suite if the editor logs anything at `error` level (CLI 方案 §五 验收要求).
static ERROR_COUNT: AtomicUsize = AtomicUsize::new(0);

fn init_logging() {
	static INIT: std::sync::Once = std::sync::Once::new();
	INIT.call_once(|| {
		struct ErrorCollector;
		impl log::Log for ErrorCollector {
			fn enabled(&self, metadata: &log::Metadata) -> bool {
				metadata.level() <= log::Level::Error
			}
			fn log(&self, record: &log::Record) {
				if record.level() == log::Level::Error {
					ERROR_COUNT.fetch_add(1, Ordering::SeqCst);
					eprintln!("[error-collector] {}: {}", record.target(), record.args());
				}
			}
			fn flush(&self) {}
		}
		let _ = log::set_boxed_logger(Box::new(ErrorCollector));
		log::set_max_level(log::LevelFilter::Error);
	});
}

/// Scratch outputs live under `target/` so the repository stays clean.
fn scratch_dir() -> PathBuf {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/tmp/graphite-cli-round-trip");
	std::fs::create_dir_all(&dir).expect("create scratch dir");
	dir
}

fn demo_artwork(filename: &str) -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../demo-artwork").join(filename)
}

/// Parse a saved `.gdd` back into its embedded legacy JSON and its resource table.
async fn read_gdd_contents(path: &Path) -> (serde_json::Value, usize) {
	let bytes = std::fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
	let container = AnyContainer::Memory(MemoryBackend::new());
	let gdd = Gdd::open_from_archive(&bytes, container, GddV1Layout)
		.await
		.unwrap_or_else(|error| panic!("open saved .gdd {}: {error}", path.display()));
	let legacy = gdd
		.read_legacy_document()
		.await
		.unwrap_or_else(|| panic!("saved .gdd {} has no embedded legacy blob", path.display()));
	let json: serde_json::Value =
		serde_json::from_slice(legacy.as_slice()).unwrap_or_else(|error| panic!("legacy blob of {} is not JSON: {error}", path.display()));
	(json, gdd.registry().resources.len())
}

fn network_nodes(document: &serde_json::Value) -> &serde_json::Value {
	&document["network_interface"]["network"]["nodes"]
}

/// The node table serializes either as a JSON object (hand-written documents) or as an array of
/// `[NodeId, node]` pairs (editor round-trip serialization). Count entries in either shape.
fn node_count(nodes: &serde_json::Value) -> usize {
	nodes.as_array().map(|entries| entries.len()).or_else(|| nodes.as_object().map(|entries| entries.len())).unwrap_or(0)
}

#[test]
fn round_trip_legacy_and_gdd() {
	init_logging();

	// Hand-rolled runtime + session slot: the session embeds a tokio runtime (the async message
	// spawner) which must be dropped outside of an async context. `catch_unwind` keeps that
	// guarantee even when an assertion fails mid-run.
	let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("build tokio runtime");
	let mut session_slot: Option<HeadlessSession> = None;

	let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
		runtime.block_on(async {
			// Park the session in the slot immediately: every later `?`/`panic!` path must leave
			// the drop to the code after `block_on`, which runs outside the async context.
			match HeadlessSession::new(scratch_dir().join("resources")).await {
				Ok(session) => session_slot.replace(session),
				Err(error) => return Err(error),
			};
			let session = session_slot.as_mut().expect("session just inserted");

			// --- Phase 1: legacy .graphite → .gdd
			let legacy_document = demo_artwork("parametric-dunescape.graphite");
			session.open_document(&legacy_document).await?;

			let out1 = scratch_dir().join("round-trip-1.gdd");
			let outcome1 = session.save_active_document_as_gdd(Some(out1.clone())).await.expect("save phase 1");
			assert!(outcome1.bytes > 0, "phase 1 save wrote no bytes");
			assert_eq!(outcome1.path, out1);
			assert!(out1.exists(), "phase 1 save did not land on disk");

			let (document1, resources1) = read_gdd_contents(&out1).await;
			let nodes1 = network_nodes(&document1);
			assert!(node_count(nodes1) > 0, "phase 1 saved document has no nodes");

			// --- Phase 2: .gdd → reopen → .gdd
			session.open_document(&out1).await.expect("reopen saved .gdd");

			let out2 = scratch_dir().join("round-trip-2.gdd");
			let outcome2 = session.save_active_document_as_gdd(Some(out2.clone())).await.expect("save phase 2");
			assert!(outcome2.bytes > 0, "phase 2 save wrote no bytes");

			let (document2, resources2) = read_gdd_contents(&out2).await;
			let nodes2 = network_nodes(&document2);

			// The hard gate: node network and resource table must survive the round trip unchanged.
			assert_eq!(nodes1, nodes2, "node network drifted across the .gdd round trip");
			assert_eq!(resources1, resources2, "resource count drifted across the .gdd round trip");

			// --- Phase 3: resave stability (.gdd → .gdd once more)
			session.open_document(&out2).await.expect("reopen resaved .gdd");

			let out3 = scratch_dir().join("round-trip-3.gdd");
			session.save_active_document_as_gdd(Some(out3.clone())).await.expect("save phase 3");

			let (document3, resources3) = read_gdd_contents(&out3).await;
			let nodes3 = network_nodes(&document3);
			assert_eq!(nodes2, nodes3, "node network drifted across the .gdd resave");
			assert_eq!(resources2, resources3, "resource count drifted across the .gdd resave");

			// Acceptance: no error-level logs, no error dialogs during the whole run.
			assert!(session.dialogs_shown.is_empty(), "editor raised dialogs: {:?}", session.dialogs_shown);
			assert_eq!(ERROR_COUNT.load(Ordering::SeqCst), 0, "editor logged error-level messages during round trip");

			Ok(())
		})
	}));

	drop(session_slot);
	drop(runtime);

	let result = match result {
		Ok(result) => result,
		Err(panic_payload) => std::panic::resume_unwind(panic_payload),
	};
	result.expect("round trip failed");
}
