mod export;

use clap::{Args, Parser, Subcommand};
use document_container::AnyContainer;
use document_container::backends::memory::MemoryBackend;
use document_format::{GddV1, GddV1Layout};
use fern::colors::{Color, ColoredLevelConfig};
use futures::executor::block_on;
use graph_craft::application_io::EditorPreferences;
use graph_craft::application_io::{PlatformApplicationIo, PlatformEditorApi};
use graph_craft::document::*;
use graph_craft::graphene_compiler::Compiler;
use graph_craft::proto::ProtoNetwork;
use graph_craft::util::load_network;
use graphene_std::application_io::{ApplicationIo, NodeGraphUpdateMessage, NodeGraphUpdateSender};
use interpreted_executor::dynamic_executor::DynamicExecutor;
use interpreted_executor::util::wrap_network_in_scope;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct UpdateLogger {}

impl NodeGraphUpdateSender for UpdateLogger {
	fn send(&self, message: NodeGraphUpdateMessage) {
		println!("{message:?}");
	}
}

#[derive(Debug, Parser)]
#[clap(name = "graphene-cli", version)]
pub struct App {
	#[clap(flatten)]
	global_opts: GlobalOpts,

	#[clap(subcommand)]
	command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
	/// Help message for compile.
	Compile {
		/// Print proto network
		#[clap(long, short = 'p')]
		print_proto: bool,

		/// Path to the .graphite document
		document: PathBuf,
	},
	/// Export a .graphite document to a file (SVG, PNG, JPG, or GIF).
	Export {
		/// Path to the .graphite document
		document: PathBuf,

		/// Output file path (extension determines format: .svg, .png, .jpg, .gif)
		#[clap(long, short = 'o')]
		output: PathBuf,

		/// Scale factor for export (default: 1.0)
		#[clap(long, default_value = "1.0")]
		scale: f64,

		/// Output width in pixels
		#[clap(long)]
		width: Option<u32>,

		/// Output height in pixels
		#[clap(long)]
		height: Option<u32>,

		/// Transparent background for PNG exports
		#[clap(long)]
		transparent: bool,

		/// Frames per second for GIF animation (default: 30)
		#[clap(long, default_value = "30")]
		fps: f64,

		/// Total number of frames for GIF animation
		#[clap(long)]
		frames: Option<u32>,

		/// Animation duration in seconds for GIF (takes precedence over --frames)
		#[clap(long)]
		duration: Option<f64>,
	},
	ListNodeIdentifiers,

	/// Inspect a .gdd or legacy .graphite document (manifest, versions, nodes, resources)
	Open {
		/// Path to the .gdd or .graphite document
		document: PathBuf,

		/// Emit machine-readable JSON instead of human-readable text
		#[clap(long)]
		json: bool,
	},

	/// Create a new .gdd document
	New {
		/// Path of the .gdd document to create
		output: PathBuf,

		/// Peer id recorded in the new document's session
		#[clap(long, default_value_t = 0)]
		peer: u64,

		/// Document uuid recorded in the new document's manifest
		#[clap(long, default_value_t = 0)]
		uuid: u64,

		/// Include a small sample node network, making the document a ready-made test fixture
		#[clap(long)]
		with_nodes: bool,
	},

	/// Extract embedded legacy .graphite file from the new .gdd file
	ExtractLegacyDoc {
		document: PathBuf,
	},
}

#[derive(Debug, Args)]
struct GlobalOpts {
	/// Verbosity level (can be specified multiple times)
	#[clap(long, short, global = true, action = clap::ArgAction::Count)]
	verbose: u8,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
	let app = App::parse();

	let log_level = app.global_opts.verbose;

	init_logging(log_level);

	match app.command {
		Command::Compile { document, print_proto } => compile_command(document, print_proto).await,
		Command::Export {
			document,
			output,
			scale,
			width,
			height,
			transparent,
			fps,
			frames,
			duration,
		} => {
			let params = ExportParams {
				document,
				output,
				scale,
				width,
				height,
				transparent,
				fps,
				frames,
				duration,
			};
			export_command(params).await
		}
		Command::ListNodeIdentifiers => list_node_identifiers_command(),
		Command::Open { document, json } => open_command(document, json).await,
		Command::New {
			output,
			peer,
			uuid,
			with_nodes,
		} => new_command(output, peer, uuid, with_nodes).await,
		Command::ExtractLegacyDoc { document } => extract_legacy_doc_command(document).await,
	}
}

/// Export subcommand parameters, collected from the clap variant into one struct.
struct ExportParams {
	document: PathBuf,
	output: PathBuf,
	scale: f64,
	width: Option<u32>,
	height: Option<u32>,
	transparent: bool,
	fps: f64,
	frames: Option<u32>,
	duration: Option<f64>,
}

/// Prints every node identifier registered in the global node metadata registry.
fn list_node_identifiers_command() -> Result<(), Box<dyn Error>> {
	let mut nodes: Vec<_> = graphene_std::registry::NODE_METADATA.lock().unwrap().keys().cloned().collect();
	nodes.sort_by_key(|x| x.as_str().to_string());
	for id in nodes {
		println!("{}", id.as_str());
	}
	Ok(())
}

/// Creates a fresh `.gdd` document at `output`: an empty working copy, or — with `with_nodes` — one
/// carrying a small sample node network, usable as a test fixture for `open` and later tooling.
async fn new_command(output: PathBuf, peer: u64, uuid: u64, with_nodes: bool) -> Result<(), Box<dyn Error>> {
	let editor_version = format!("graphene-cli-{}", env!("CARGO_PKG_VERSION"));
	let stdlib_version = format!("graphene-std-{}", env!("CARGO_PKG_VERSION"));

	// `GddV1::create` is folder-backed (native-only): stage the working copy in a scratch directory,
	// then export the single-file `.gdd` archive from it.
	let staging = std::env::temp_dir().join(format!("graphene-cli-new-{}-{uuid}", std::process::id()));
	let _ = std::fs::remove_dir_all(&staging);

	let mut gdd = GddV1::create(&staging, document_graph_storage::PeerId(peer), uuid, editor_version, stdlib_version)
		.await
		.map_err(|error| format!("Failed to create document: {error}"))?;

	// The byte store only needs to live for this process: `export_to_bytes` re-materializes embedded
	// resources (the proto-node declaration bytes) from it into the archive.
	let byte_store = graph_craft::application_io::resource::HashMapResourceStorage::new();

	if with_nodes {
		let network = sample_network();
		gdd.commit_from_runtime(&network, &document_graph_storage::NoMetadata, &graphene_resource::ResourceRegistry::new(), &byte_store)
			.map_err(|error| format!("Failed to commit sample nodes: {error}"))?;
	}

	let bytes = gdd
		.export_to_bytes(document_format::ExportFormat::Zip, document_format::ExportOptions::default(), &byte_store, None)
		.await
		.map_err(|error| format!("Failed to export document: {error}"))?;

	if let Some(parent) = output.parent() {
		if !parent.as_os_str().is_empty() {
			std::fs::create_dir_all(parent).map_err(|error| format!("Failed to create output directory {}: {error}", parent.display()))?;
		}
	}
	std::fs::write(&output, bytes).map_err(|error| format!("Failed to write document {}: {error}", output.display()))?;

	let _ = std::fs::remove_dir_all(&staging);

	eprintln!("Created {}", output.display());
	Ok(())
}

/// A small sample network of three chained identity proto-nodes (`import → 0 → 1 → 2 → export`),
/// deterministic in shape so generated fixtures are comparable across runs.
fn sample_network() -> NodeNetwork {
	use graph_craft::ProtoNodeIdentifier;
	use graph_craft::concrete;
	use graph_craft::document::{DocumentNode, DocumentNodeImplementation};

	let identity = |id: NodeId, input: NodeInput| {
		(
			id,
			DocumentNode {
				inputs: vec![input],
				implementation: DocumentNodeImplementation::ProtoNode(ProtoNodeIdentifier::new("graphene_core::ops::identity::IdentityNode")),
				..Default::default()
			},
		)
	};

	NodeNetwork {
		exports: vec![NodeInput::node(NodeId(2), 0)],
		nodes: [
			identity(NodeId(0), NodeInput::import(concrete!(u32), 0)),
			identity(NodeId(1), NodeInput::node(NodeId(0), 0)),
			identity(NodeId(2), NodeInput::node(NodeId(1), 0)),
		]
		.into_iter()
		.collect(),
		..Default::default()
	}
}

/// Inspects a `.gdd` archive or a legacy `.graphite` document: manifest, versions, node list, and
/// resource list. Human-readable on stdout by default; `--json` switches to machine-readable JSON.
async fn open_command(document_path: PathBuf, json: bool) -> Result<(), Box<dyn Error>> {
	let gdd = open_gdd_for_path(&document_path).await?;
	let report = match gdd {
		Some(gdd) => inspect_gdd(gdd, &document_path).await?,
		None => inspect_legacy(&document_path)?,
	};

	if json {
		let text = serde_json::to_string_pretty(&report).map_err(|error| format!("Failed to serialize inspection report: {error}"))?;
		println!("{text}");
	} else {
		print!("{}", render_report(&report));
	}
	Ok(())
}

/// Builds the inspection report for a `.gdd` document: manifest, session cursors, networks, the
/// node list (proto-node identifiers resolved through the declaration resources), the resource
/// list, and whether a legacy `.graphite` blob is embedded.
async fn inspect_gdd(gdd: GddV1, document_path: &Path) -> Result<serde_json::Value, Box<dyn Error>> {
	let declarations = gdd.declarations(&gdd).await;
	let registry = gdd.registry();
	let manifest = gdd.manifest();
	let session = gdd.session();

	let mut node_ids: Vec<_> = registry.node_instances.keys().copied().collect();
	node_ids.sort_unstable();
	let nodes: Vec<serde_json::Value> = node_ids
		.iter()
		.map(|id| {
			let node = &registry.node_instances[id];
			let implementation = match node.implementation() {
				document_graph_storage::Implementation::ProtoNode(resource_id) => declarations
					.get(resource_id)
					.map(|proto| proto.identifier.clone())
					.unwrap_or_else(|| format!("<unresolved proto-node resource {resource_id:?}>")),
				document_graph_storage::Implementation::Network(network_id) => format!("<network {network_id:?}>"),
			};
			serde_json::json!({
				"id": id.0,
				"implementation": implementation,
				"network": node.network().0,
			})
		})
		.collect();

	let mut network_ids: Vec<_> = registry.networks.keys().copied().collect();
	network_ids.sort_unstable();

	let mut resource_ids: Vec<_> = registry.resources.keys().copied().collect();
	resource_ids.sort_unstable();
	let resources: Vec<serde_json::Value> = resource_ids
		.iter()
		.map(|id| {
			let entry = &registry.resources[id];
			serde_json::json!({
				"id": format!("{id:?}"),
				"hash": entry.hash.map(|hash| hash.to_string()),
				"sources": entry.sources.len(),
			})
		})
		.collect();

	let legacy = gdd.read_legacy_document().await;

	Ok(serde_json::json!({
		"kind": "gdd",
		"path": document_path.display().to_string(),
		"manifest": {
			"format": manifest.format,
			"format_version": manifest.format_version,
			"editor_version": manifest.editor_version,
			"stdlib_version": manifest.stdlib_version,
			"document_id": manifest.document_id,
			"codecs": serde_json::to_value(manifest.codecs).map_err(|error| format!("Failed to serialize codecs: {error}"))?,
		},
		"session": {
			"peer": session.peer().0,
			"head_rev": session.head_rev().map(|rev| rev.0.to_string()),
			"history_entries": session.history().count(),
			"hot_log_entries": session.hot_log().len(),
			"next_node_counter": session.next_node_counter(),
		},
		"networks": network_ids.iter().map(|id| id.0).collect::<Vec<_>>(),
		"nodes": nodes,
		"resources": resources,
		"legacy_embedded": match legacy {
			Some(bytes) => serde_json::json!({ "present": true, "bytes": bytes.as_slice().len() }),
			None => serde_json::json!({ "present": false }),
		},
	}))
}

/// Builds the inspection report for a legacy `.graphite` document, parsing the network through the
/// existing `load_network` path.
fn inspect_legacy(document_path: &Path) -> Result<serde_json::Value, Box<dyn Error>> {
	let document_string = std::fs::read_to_string(document_path).map_err(|error| format!("Failed to read document {}: {error}", document_path.display()))?;
	let document: serde_json::Value = serde_json::from_str(&document_string).map_err(|error| format!("Failed to parse document {}: {error}", document_path.display()))?;

	// The existing load path used by `compile`/`export`.
	let network = load_network(&document_string);

	let mut node_ids: Vec<_> = network.nodes.keys().copied().collect();
	node_ids.sort_unstable();
	let nodes: Vec<serde_json::Value> = node_ids
		.iter()
		.map(|id| {
			let node = &network.nodes[id];
			serde_json::json!({
				"id": id.0,
				"implementation": document_implementation_label(&node.implementation),
			})
		})
		.collect();

	Ok(serde_json::json!({
		"kind": "legacy",
		"path": document_path.display().to_string(),
		"commit_hash": document["commit_hash"].as_str().unwrap_or_default(),
		"nodes": nodes,
	}))
}

/// Short label for a legacy document node's implementation.
fn document_implementation_label(implementation: &DocumentNodeImplementation) -> String {
	match implementation {
		DocumentNodeImplementation::ProtoNode(identifier) => identifier.as_str().to_string(),
		DocumentNodeImplementation::Network(_) => "<network>".to_string(),
		other => format!("{other:?}"),
	}
}

/// Renders an inspection report as human-readable text. The JSON report is the authoritative shape;
/// this only formats it.
fn render_report(report: &serde_json::Value) -> String {
	let mut out = String::new();
	out.push_str(&format!("Document: {}\n", report["path"].as_str().unwrap_or("?")));
	out.push_str(&format!("Kind:     {}\n", report["kind"].as_str().unwrap_or("?")));

	let manifest = &report["manifest"];
	if manifest.is_object() {
		out.push_str("\nManifest\n");
		out.push_str(&format!("  format:          {}\n", manifest["format"].as_str().unwrap_or("?")));
		out.push_str(&format!("  format_version:  {}\n", manifest["format_version"]));
		out.push_str(&format!("  editor_version:  {}\n", manifest["editor_version"].as_str().unwrap_or("?")));
		out.push_str(&format!("  stdlib_version:  {}\n", manifest["stdlib_version"].as_str().unwrap_or("?")));
		out.push_str(&format!("  document_id:     {}\n", manifest["document_id"]));
		let codecs = &manifest["codecs"];
		if codecs.is_object() {
			let codec = |key: &str| codecs[key].as_str().unwrap_or("?");
			out.push_str(&format!(
				"  codecs:          registry={}, history={}, hot_log={}, session={}\n",
				codec("registry"),
				codec("history"),
				codec("hot_log"),
				codec("session")
			));
		}
	}

	let session = &report["session"];
	if session.is_object() {
		out.push_str("\nSession\n");
		out.push_str(&format!("  peer:              {}\n", session["peer"]));
		out.push_str(&format!("  head_rev:          {}\n", session["head_rev"].as_str().unwrap_or("none")));
		out.push_str(&format!("  history_entries:   {}\n", session["history_entries"]));
		out.push_str(&format!("  hot_log_entries:   {}\n", session["hot_log_entries"]));
		out.push_str(&format!("  next_node_counter: {}\n", session["next_node_counter"]));
	}

	if let Some(networks) = report["networks"].as_array() {
		out.push_str(&format!("\nNetworks ({})\n", networks.len()));
		for network in networks {
			out.push_str(&format!("  {network}\n"));
		}
	}

	if let Some(nodes) = report["nodes"].as_array() {
		out.push_str(&format!("\nNodes ({})\n", nodes.len()));
		for node in nodes {
			let implementation = node["implementation"].as_str().unwrap_or("?");
			// Only `.gdd` documents track a per-node storage network; legacy nodes have none.
			let network = match node["network"].as_u64() {
				Some(network) => format!(" [network {network}]"),
				None => String::new(),
			};
			out.push_str(&format!("  {}: {implementation}{network}\n", node["id"]));
		}
	}

	if let Some(resources) = report["resources"].as_array() {
		out.push_str(&format!("\nResources ({})\n", resources.len()));
		for resource in resources {
			let hash = resource["hash"].as_str().map(|hash| format!("hash={hash} ")).unwrap_or_default();
			let id = resource["id"].as_str().unwrap_or("?");
			out.push_str(&format!("  {id}: {hash}sources={}\n", resource["sources"]));
		}
	}

	let legacy = &report["legacy_embedded"];
	if legacy.is_object() {
		if legacy["present"].as_bool() == Some(true) {
			out.push_str(&format!("\nLegacy .graphite: embedded ({} bytes)\n", legacy["bytes"]));
		} else {
			out.push_str("\nLegacy .graphite: not embedded\n");
		}
	}

	let commit_hash = report["commit_hash"].as_str().unwrap_or_default();
	if !commit_hash.is_empty() {
		out.push_str(&format!("\nCommit hash: {commit_hash}\n"));
	}

	out
}

/// Extracts the embedded legacy `.graphite` document from a `.gdd` file, writing it next to the
/// `.gdd` with the extension swapped to `.graphite`.
async fn extract_legacy_doc_command(document_path: PathBuf) -> Result<(), Box<dyn Error>> {
	let gdd = open_gdd_for_path(&document_path).await?;
	let Some(gdd) = gdd else {
		return Err("ExtractLegacyDoc requires a .gdd document".into());
	};
	let Some(legacy_doc) = gdd.read_legacy_document().await else {
		return Err("gdd file did not contain a legacy .graphite document".into());
	};
	let mut new_path = document_path.clone();
	new_path.set_extension("graphite");
	std::fs::write(&new_path, legacy_doc).map_err(|error| format!("Failed to write .graphite file: {error}"))?;
	eprintln!("Saved file to {}", new_path.to_string_lossy());
	Ok(())
}

/// Compiles the document; with `--print-proto` the compiled proto network is printed to stdout.
async fn compile_command(document_path: PathBuf, print_proto: bool) -> Result<(), Box<dyn Error>> {
	let compiled = load_and_compile(&document_path).await?;

	if print_proto {
		println!("{}", compiled.proto_graph);
	}

	Ok(())
}

/// Loads, compiles, and renders the document to the output file (SVG, PNG, JPG, or GIF).
async fn export_command(params: ExportParams) -> Result<(), Box<dyn Error>> {
	let compiled = load_and_compile(&params.document).await?;

	// Spawn thread to poll GPU device
	let device = compiled.application_io.gpu_executor().unwrap().context().device.clone();
	std::thread::spawn(move || {
		loop {
			std::thread::sleep(std::time::Duration::from_nanos(10));
			device.poll(wgpu::PollType::Poll).unwrap();
		}
	});

	// Detect output file type
	let file_type = export::detect_file_type(&params.output)?;

	// Create executor
	let executor = create_executor(compiled.proto_graph)?;

	if params.fps <= 0. {
		return Err("Fps number must be positive".into());
	}

	let wgpu_executor_ref = compiled.application_io.gpu_executor().unwrap();

	// Perform export based on file type
	if file_type == export::FileType::Gif {
		let animation = export::AnimationParams::new(params.fps, params.frames, params.duration);
		export::export_gif(&executor, wgpu_executor_ref, params.output, params.scale, (params.width, params.height), animation).await?;
	} else {
		export::export_document(&executor, wgpu_executor_ref, params.output, file_type, params.scale, (params.width, params.height), params.transparent).await?;
	}

	Ok(())
}

/// A compiled document plus the GPU application context it depends on. The wgpu executor borrows
/// from `application_io`, so this struct must outlive any use of the executor reference.
struct CompiledDocument {
	proto_graph: ProtoNetwork,
	application_io: Arc<PlatformApplicationIo>,
}

/// Loads the document at `document_path` (`.gdd` archive or legacy `.graphite`), builds the GPU
/// application context, and compiles the node network into a proto network.
async fn load_and_compile(document_path: &Path) -> Result<CompiledDocument, Box<dyn Error>> {
	let gdd = open_gdd_for_path(document_path).await?;

	// Build the runtime network: from the `.gdd` registry, or by loading a legacy `.graphite` document.
	let node_network = match &gdd {
		Some(gdd) => {
			let declarations = gdd.declarations(gdd).await;
			let (node_network, _metadata) = gdd.registry().to_runtime_with_metadata(&declarations)?;
			node_network
		}
		None => {
			let document_string = std::fs::read_to_string(document_path).map_err(|error| format!("Failed to read document {}: {error}", document_path.display()))?;
			load_network(&document_string)
		}
	};

	log::info!("Creating GPU context");
	let mut application_io = PlatformApplicationIo::new().await;
	if let Some(gdd) = &gdd {
		application_io.inject_resource_proxy(Box::new(gdd.resource_proxy()));
	}
	let application_io = Arc::new(application_io);

	let preferences = EditorPreferences {
		max_render_region_size: EditorPreferences::default().max_render_region_size,
	};
	let editor_api = Arc::new(PlatformEditorApi {
		application_io: Some(application_io.clone()),
		node_graph_message_sender: Box::new(UpdateLogger {}),
		editor_preferences: Box::new(preferences),
	});
	let proto_graph = compile_graph(node_network, editor_api, gdd.as_ref())?;

	Ok(CompiledDocument {
		proto_graph,
		application_io,
	})
}

/// Opens a `.gdd` document when `document_path` carries a `.gdd` extension; returns `None` for any
/// other extension, which is treated as a legacy `.graphite` document (no `Gdd`, so resources fall
/// back to the default registry).
async fn open_gdd_for_path(document_path: &Path) -> Result<Option<GddV1>, Box<dyn Error>> {
	let is_gdd = document_path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("gdd"));

	if !is_gdd {
		return Ok(None);
	}

	let archive = std::fs::read(document_path).map_err(|error| format!("Failed to read document {}: {error}", document_path.display()))?;
	let container = AnyContainer::Memory(MemoryBackend::new());
	let gdd = document_format::Gdd::open_from_archive(archive.as_ref(), container, GddV1Layout)
		.await
		.map_err(|error| format!("Failed to open document: {error}"))?;
	Ok(Some(gdd))
}

fn init_logging(log_level: u8) {
	let default_level = match log_level {
		0 => log::LevelFilter::Error,
		1 => log::LevelFilter::Info,
		2 => log::LevelFilter::Debug,
		_ => log::LevelFilter::Trace,
	};
	let colors = ColoredLevelConfig::new().debug(Color::Magenta).info(Color::Green).error(Color::Red);
	fern::Dispatch::new()
		.chain(std::io::stdout())
		.level_for("wgpu", log::LevelFilter::Error)
		.level_for("naga", log::LevelFilter::Error)
		.level_for("wgpu_hal", log::LevelFilter::Error)
		.level_for("wgpu_core", log::LevelFilter::Error)
		.level(default_level)
		.format(move |out, message, record| {
			out.finish(format_args!(
				"[{}]{}{} {}",
				// This will color the log level only, not the whole line. Just a touch.
				colors.color(record.level()),
				chrono::Utc::now().format("[%Y-%m-%d %H:%M:%S]"),
				record.module_path().unwrap_or(""),
				message
			))
		})
		.apply()
		.unwrap();
}

fn compile_graph(network: NodeNetwork, editor_api: Arc<PlatformEditorApi>, gdd: Option<&GddV1>) -> Result<ProtoNetwork, Box<dyn Error>> {
	let preprocessor = preprocessor::Preprocessor::new();

	let mut network = wrap_network_in_scope(network, editor_api);

	// A `.gdd` resolves resource hashes from its registry; a legacy `.graphite` has no resource store, so it
	// preprocesses against an empty registry (matching the pre-`.gdd` CLI behavior).
	match gdd {
		Some(gdd) => preprocessor
			.preprocess(&mut network, &|resource_id| gdd.registry().resources.get(&resource_id).and_then(|r| r.hash))
			.expect("Failed to expand network"),
		None => { preprocessor.preprocess(&mut network, &|_| None) }.expect("Failed to expand network"),
	}

	let compiler = Compiler {};
	compiler.compile_single(network).map_err(|x| x.into())
}

fn create_executor(proto_network: ProtoNetwork) -> Result<DynamicExecutor, Box<dyn Error>> {
	let executor = block_on(DynamicExecutor::new(proto_network)).map_err(|errors| errors.iter().map(|e| format!("{e:?}")).reduce(|acc, e| format!("{acc}\n{e}")).unwrap_or_default())?;
	Ok(executor)
}
