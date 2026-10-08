//! Acceptance tests for the three Rust primitives (CLI 方案 v2: inspect / exec / export).
//!
//! Also the home of the **measured serde shapes** deliverable: the `SHAPE` lines printed by
//! `measured_message_serde_shapes` are the source of truth recorded in `cli/docs/message-shapes.md`.
//!
//! The editor `ENVIRONMENT` is a process-wide `OnceLock` (`editor/src/application.rs:21`), so all
//! editor-dependent tests share one session inside a single test function. **MUST** be run with
//! `--test-threads=1`.

use graphite_cli::exec;
use graphite_cli::export;
use graphite_cli::inspect::{self, InspectSource};
use graphite_cli::session::HeadlessSession;
use graphite_editor::messages::message::Message;
use graphite_editor::messages::portfolio::document::node_graph::document_node_definitions::DefinitionIdentifier;
use graphite_editor::messages::portfolio::document::utility_types::document_metadata::LayerNodeIdentifier;
use graphite_editor::messages::prelude::*;
use std::path::PathBuf;

fn scratch_dir() -> PathBuf {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/tmp/graphite-cli-primitives");
	std::fs::create_dir_all(&dir).expect("create scratch dir");
	dir
}

fn testfiles(filename: &str) -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../testfiles").join(filename)
}

// ================================================================================================
// Deliverable: measured serde shapes (pure serde, no editor session)
// ================================================================================================

/// Prints and round-trip-verifies the exact JSON shapes agents must emit for `exec`.
/// The `SHAPE` lines are transcribed into `cli/docs/message-shapes.md`.
#[test]
fn measured_message_serde_shapes() {
	// NodeGraphMessage nesting as prescribed by the contract:
	// {"Portfolio":{"Document":{"NodeGraph":{"SetInputValue":{…}}}}}
	let set_param = Message::from(NodeGraphMessage::SetInputValue {
		node_id: graph_craft::document::NodeId(333),
		input_index: 2,
		value: Box::new(graph_craft::document::value::TaggedValue::F64(42.5)),
	});
	let select = Message::from(NodeGraphMessage::SelectedNodesSet { nodes: vec![graph_craft::document::NodeId(1), graph_craft::document::NodeId(2)] });
	let preference = Message::from(PreferencesMessage::SaveAsGdd { enabled: true });

	for message in [set_param, select, preference] {
		let json = serde_json::to_string(&message).expect("message serializes");
		println!("SHAPE {json}");
		let back: Message = serde_json::from_str(&json).expect("message deserializes");
		assert_eq!(serde_json::to_string(&back).unwrap(), json, "round-trip mismatch");
	}

	// Value-level shapes.
	let cases: Vec<(&str, String)> = vec![
		("DAffine2", serde_json::to_string(&glam::DAffine2::from_cols_array(&[2.0, 0.0, 10.0, 0.0, 3.0, 20.0])).unwrap()),
		("TaggedValue::Bool", serde_json::to_string(&graph_craft::document::value::TaggedValue::Bool(true)).unwrap()),
		("TaggedValue::F32", serde_json::to_string(&graph_craft::document::value::TaggedValue::F32(1.25)).unwrap()),
		("TaggedValue::F64", serde_json::to_string(&graph_craft::document::value::TaggedValue::F64(42.5)).unwrap()),
		("TaggedValue::U32", serde_json::to_string(&graph_craft::document::value::TaggedValue::U32(7)).unwrap()),
		("TaggedValue::String", serde_json::to_string(&graph_craft::document::value::TaggedValue::String("hi".into())).unwrap()),
		("TaggedValue::None", serde_json::to_string(&graph_craft::document::value::TaggedValue::None).unwrap()),
		(
			"TaggedValue::LegacyOptionalDAffine2(Some)",
			serde_json::to_string(&graph_craft::document::value::TaggedValue::LegacyOptionalDAffine2(Some(glam::DAffine2::IDENTITY))).unwrap(),
		),
		(
			"TaggedValue::LegacyOptionalDAffine2(None)",
			serde_json::to_string(&graph_craft::document::value::TaggedValue::LegacyOptionalDAffine2(None)).unwrap(),
		),
		("DefinitionIdentifier::Network", serde_json::to_string(&DefinitionIdentifier::Network("Rectangle".to_string())).unwrap()),
		(
			"DefinitionIdentifier::ProtoNode",
			serde_json::to_string(&DefinitionIdentifier::ProtoNode(graph_craft::ProtoNodeIdentifier::new("graphene_core::ops::identity::IdentityNode"))).unwrap(),
		),
		("LayerNodeIdentifier::ROOT_PARENT", serde_json::to_string(&LayerNodeIdentifier::ROOT_PARENT).unwrap()),
		("LayerNodeIdentifier(NodeId 7)", serde_json::to_string(&LayerNodeIdentifier::new_unchecked(graph_craft::document::NodeId(7))).unwrap()),
	];
	for (name, json) in cases {
		println!("SHAPE {name} -> {json}");
	}

	// Round-trips for the enum-ish value shapes.
	for (json, target) in [
		(serde_json::to_string(&LayerNodeIdentifier::ROOT_PARENT).unwrap(), "LayerNodeIdentifier"),
		(serde_json::to_string(&LayerNodeIdentifier::new_unchecked(graph_craft::document::NodeId(7))).unwrap(), "LayerNodeIdentifier"),
	] {
		match target {
			"LayerNodeIdentifier" => {
				let back: LayerNodeIdentifier = serde_json::from_str(&json).expect("LayerNodeIdentifier round-trips");
				assert_eq!(serde_json::to_string(&back).unwrap(), json);
			}
			_ => unreachable!(),
		}
	}

	// Pitfall §四.5: a message with serde(skip) payload fields must fail deserialization with a
	// clear error (never a panic). `UpdateTypes` is not a FrontendMessage variant in this editor
	// version, so the JSON form is rejected as an unknown variant.
	let bad = r#"[{"Frontend":{"UpdateTypes":{"types":[]}}}]"#;
	match serde_json::from_str::<Vec<Message>>(bad) {
		Ok(_) => panic!("UpdateTypes unexpectedly deserialized — update the message-shapes docs"),
		Err(error) => println!("SHAPE UpdateTypes -> rejected: {}", error.to_string().split(',').next().unwrap_or("").trim()),
	}
}

// ================================================================================================
// Primitive acceptance (one editor per process: single test, --test-threads=1)
// ================================================================================================

#[test]
fn primitives_acceptance() {
	static ERROR_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
	let init_logging = || {
		static INIT: std::sync::Once = std::sync::Once::new();
		INIT.call_once(|| {
			struct ErrorCollector;
			impl log::Log for ErrorCollector {
				fn enabled(&self, metadata: &log::Metadata) -> bool {
					metadata.level() <= log::Level::Error
				}
				fn log(&self, record: &log::Record) {
					if record.level() == log::Level::Error {
						ERROR_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
						eprintln!("[error-collector] {}: {}", record.target(), record.args());
					}
				}
				fn flush(&self) {}
			}
			let _ = log::set_boxed_logger(Box::new(ErrorCollector));
			log::set_max_level(log::LevelFilter::Error);
		});
	};
	init_logging();

	let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("build tokio runtime");
	let mut session_slot: Option<HeadlessSession> = None;

	let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
		runtime.block_on(async {
			match HeadlessSession::new(scratch_dir().join("resources")).await {
				Ok(session) => session_slot.replace(session),
				Err(error) => return Err(error),
			};
			let session = session_slot.as_mut().expect("session just inserted");

			// ==== inspect primitive: the three fixed samples ====
			// 1) parametric-dunescape.gdd: a real editor-save artifact → editor read path.
			let dunescape = testfiles("parametric-dunescape.gdd");
			session.open_document(&dunescape).await?;
			let snapshot = session.capture_document_json().await?;
			let inspected = inspect::inspect_editor_snapshot(&snapshot)?;
			assert_eq!(inspected.source, InspectSource::Editor);
			assert_eq!(inspected.node_count, 37, "dunescape node count");
			assert!(!inspected.wires.is_empty(), "dunescape must have wires");
			assert!(!inspected.layers.is_empty(), "dunescape must have a layer tree");
			println!(
				"INSPECT parametric-dunescape.gdd: {} nodes, {} wires, {} root layers (source={:?})",
				inspected.node_count,
				inspected.wires.len(),
				inspected.layers.len(),
				inspected.source
			);
			let dunescape_inspected = inspected;

			// 2) + 3) L1 compiler-stack fixtures the editor cannot open → registry read path.
			let empty = inspect::inspect_registry_network(&inspect::read_registry_network(&testfiles("sample-empty.gdd")).await.expect("empty fixture"));
			assert_eq!(empty.source, InspectSource::Registry);
			assert_eq!(empty.node_count, 0, "sample-empty.gdd must inspect as empty");
			assert!(empty.wires.is_empty());
			println!("INSPECT sample-empty.gdd: empty (source=registry)");

			let nodes_fixture =
				inspect::inspect_registry_network(&inspect::read_registry_network(&testfiles("sample-nodes.gdd")).await.expect("nodes fixture"));
			assert_eq!(nodes_fixture.source, InspectSource::Registry);
			assert_eq!(nodes_fixture.node_count, 3, "sample-nodes.gdd must inspect as 3 nodes");
			assert_eq!(nodes_fixture.wires.len(), 2, "sample-nodes.gdd chains 3 identity nodes with 2 wires");
			for node in &nodes_fixture.nodes {
				assert_eq!(node.identifier.as_deref(), Some("graphene_core::ops::identity::IdentityNode"), "identity fixture node type");
			}
			println!("INSPECT sample-nodes.gdd: 3 nodes, 2 wires (source=registry)");

			// ==== exec primitive: SetInputValue + SelectedNodesSet injection ====
			// Find a node with an editable scalar (Value) parameter in the dunescape document.
			// `params` values are raw `TaggedValue` JSON (externally tagged), so the new value is
			// built with the same variant as the old one.
			let (target_node, target_input, variant, old_inner) = dunescape_inspected
				.nodes
				.iter()
				.find_map(|node| {
					node.params.iter().find_map(|(index, value)| {
						let (variant, inner) = value.as_object()?.into_iter().next()?;
						let supported = matches!(
							variant.as_str(),
							"F64" | "F32" | "U32" | "U64" | "Bool" | "String"
						) && matches!(inner, serde_json::Value::Number(_) | serde_json::Value::Bool(_) | serde_json::Value::String(_));
						supported.then(|| (node.id, index.clone(), variant, inner.clone()))
					})
				})
				.expect("dunescape must have at least one scalar value parameter");

			let new_value = match (variant.as_str(), &old_inner) {
				("F64", serde_json::Value::Number(number)) => serde_json::json!({ "F64": number.as_f64().expect("f64") + 11.0 }),
				("F32", serde_json::Value::Number(number)) => serde_json::json!({ "F32": (number.as_f64().expect("f32") + 11.0) as f32 }),
				("U32", serde_json::Value::Number(number)) => serde_json::json!({ "U32": number.as_u64().expect("u32") + 11 }),
				("U64", serde_json::Value::Number(number)) => serde_json::json!({ "U64": number.as_u64().expect("u64") + 11 }),
				("Bool", serde_json::Value::Bool(value)) => serde_json::json!({ "Bool": !value }),
				("String", serde_json::Value::String(text)) => serde_json::json!({ "String": format!("{text}!") }),
				_ => unreachable!("the finder only yields the variants handled here"),
			};
			let old_value = serde_json::json!({ variant: old_inner });

			// `exec`/`export` cannot process the L1 fixtures either: the editor would panic inside
			// graph flattening (graph-craft document.rs:980, measured). The CLI probes up front
			// and rejects them with a clean error — verified here without touching the editor
			// (a panic would kill the shared session).
			assert_eq!(
				inspect::probe_gdd_document(&testfiles("sample-nodes.gdd")).await.expect("probe"),
				inspect::DocumentProbe::RegistryOnly,
				"sample-nodes.gdd must be detected as editor-unopenable"
			);
			assert_eq!(
				inspect::probe_gdd_document(&testfiles("sample-empty.gdd")).await.expect("probe"),
				inspect::DocumentProbe::RegistryOnly,
				"sample-empty.gdd must be detected as editor-unopenable"
			);
			assert_eq!(
				inspect::probe_gdd_document(&testfiles("parametric-dunescape.gdd")).await.expect("probe"),
				inspect::DocumentProbe::EditorBacked,
				"dunescape must keep using the editor read path"
			);
			println!("EXEC sample-nodes.gdd/sample-empty.gdd: rejected up front as compiler-stack .gdd (clean error, no editor panic)");

			// The injection itself runs against the openable dunescape document. Built through
			// serde so the JSON exactly matches the measured externally-tagged shapes.
			session.open_document(&dunescape).await?;
			let injection = serde_json::to_string(&serde_json::json!([
				{"Portfolio": {"Document": {"NodeGraph": {"SetInputValue": {
					"node_id": target_node, "input_index": target_input.parse::<u64>().expect("input index"), "value": new_value
				}}}}},
				{"Portfolio": {"Document": {"NodeGraph": {"SelectedNodesSet": {"nodes": [target_node]}}}}},
			]))
			.expect("injection serializes");
			let messages: Vec<Message> = serde_json::from_str(&injection).expect("injection parses");
			assert_eq!(messages.len(), 2);
			let summary = exec::execute(session, messages, None).await?;
			assert_eq!(summary.error_count, 0, "injection must run clean: {:?}", summary.events);
			assert!(!summary.events.is_empty(), "injection must produce interceptor events");
			println!("EXEC injected SetInputValue+SelectedNodesSet: {} events, 0 errors", summary.events.len());

			// Re-inspect the in-memory editor state (no reopen — reopening would reload the file
			// from disk and discard the in-memory injection).
			let snapshot = session.capture_document_json().await?;
			let re_inspected = inspect::inspect_editor_snapshot(&snapshot)?;
			let changed = re_inspected
				.nodes
				.iter()
				.find(|node| node.id == target_node)
				.expect("target node still present")
				.params
				.get(&target_input)
				.expect("target input still present")
				.clone();
			assert_eq!(changed, new_value, "parameter must equal the injected value after exec");
			assert_ne!(changed, old_value, "parameter must differ from the pre-injection value");
			println!("EXEC verified: node {target_node} input {target_input} = {changed}");

			// exec --save from a **cold** open (regression for the storage-drift bug: a bare
			// batch once saved a drifted .gdd — legacy blob carried the new value while the
			// graph-storage registry kept the old one, rolling back on reopen). The batch is
			// transaction-wrapped now; the reopen below must see the injected value.
			let saved = scratch_dir().join("exec-saved.gdd");
			session.open_document(&dunescape).await?;
			let messages: Vec<Message> = serde_json::from_str(&injection).expect("injection parses");
			let summary = exec::execute(session, messages, Some(&saved)).await?;
			assert_eq!(summary.error_count, 0);
			assert_eq!(summary.saved_path.as_deref(), Some(saved.to_string_lossy().as_ref()));
			assert!(saved.exists(), "exec --save must write the file");

			session.open_document(&saved).await?;
			let snapshot = session.capture_document_json().await?;
			let saved_inspected = inspect::inspect_editor_snapshot(&snapshot)?;
			let persisted = saved_inspected
				.nodes
				.iter()
				.find(|node| node.id == target_node)
				.expect("target node in saved artifact")
				.params
				.get(&target_input)
				.expect("target input in saved artifact")
				.clone();
			assert_eq!(persisted, new_value, "injected value must survive save + reopen (no storage drift)");
			println!("EXEC --save verified: {} carries the injected value after reopen", saved.display());

			// ==== export primitive: SVG + PNG through the editor chain ====
			let svg_out = scratch_dir().join("dunescape.svg");
			let svg_summary = export::run(session, &dunescape, &svg_out, None, None, 1.0, false).await?;
			assert!(svg_summary.bytes > 0, "SVG export must be non-empty");
			let svg_text = std::fs::read_to_string(&svg_out).expect("svg readable");
			assert!(svg_text.contains("<svg"), "SVG must have an <svg> root");
			assert!(svg_text.contains("<path"), "SVG must contain non-empty path elements");
			assert!(svg_summary.width.unwrap_or(0) > 0 && svg_summary.height.unwrap_or(0) > 0, "SVG dimensions must parse");
			println!("EXPORT svg: {} bytes, {}x{}", svg_summary.bytes, svg_summary.width.unwrap(), svg_summary.height.unwrap());

			let png_out = scratch_dir().join("dunescape.png");
			let png_summary = export::run(session, &dunescape, &png_out, None, None, 1.0, false).await?;
			assert!(png_summary.bytes > 0, "PNG export must be non-empty");
			let png_bytes = std::fs::read(&png_out).expect("png readable");
			let reader = image::ImageReader::new(std::io::Cursor::new(&png_bytes))
				.with_guessed_format()
				.expect("png format guess")
				.into_dimensions()
				.expect("PNG must open via the image crate");
			assert_eq!((reader.0, reader.1), (png_summary.width.unwrap(), png_summary.height.unwrap()), "PNG dimensions must match the summary");
			assert!(reader.0 > 0 && reader.1 > 0);
			println!("EXPORT png: {} bytes, {}x{}", png_summary.bytes, reader.0, reader.1);

			// --width override: the corrective pass must produce exactly the requested width.
			let png_width_out = scratch_dir().join("dunescape-1024.png");
			let png_width_summary = export::run(session, &dunescape, &png_width_out, Some(1024), None, 1.0, false).await?;
			assert_eq!(png_width_summary.width, Some(1024), "--width must be honored exactly");
			let width_png = std::fs::read(&png_width_out).expect("png readable");
			let dimensions = image::ImageReader::new(std::io::Cursor::new(&width_png))
				.with_guessed_format()
				.expect("png format guess")
				.into_dimensions()
				.expect("PNG must decode");
			assert_eq!(dimensions.0, 1024, "decoded PNG width must equal --width");
			println!("EXPORT png --width 1024: {}x{}", dimensions.0, dimensions.1);

			assert_eq!(ERROR_COUNT.load(std::sync::atomic::Ordering::SeqCst), 0, "error-level logs during primitive flows");

			Ok(())
		})
	}));

	drop(session_slot);
	drop(runtime);

	let result = match result {
		Ok(result) => result,
		Err(panic_payload) => std::panic::resume_unwind(panic_payload),
	};
	result.expect("primitive acceptance failed");
}
