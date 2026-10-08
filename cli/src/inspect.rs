//! Document structure inspection (CLI 原语 `inspect`).
//!
//! Two read paths feed the same contract output:
//!
//! 1. **Editor snapshot** (`inspect_editor_snapshot`): the JSON produced by the editor's own save
//!    pipeline (`HeadlessSession::capture_document_json`). It is the editor's authoritative view
//!    of the document network — read directly from the serialized `document_network` instead of
//!    waiting for frontend messages, which are gated on UI state like `graph_view_overlay_open`
//!    and never arrive in a headless session (CLI 方案 坑#5).
//! 2. **Registry fallback** (`inspect_registry_network`): `.gdd` files produced by the compiler
//!    stack (`graphene-cli new --with-nodes` fixtures) have no embedded editor document, so the
//!    editor cannot open them (measured: empty registry → "Network 0 not found"; identity-chain
//!    fixtures fail inside graph flattening). For those, the graph-storage registry is converted
//!    back into a `NodeNetwork` and inspected read-only. Such output is tagged `"source":
//!    "registry"` and carries no editor-side metadata (positions/layers).
//!
//! Output contract (CLI 接口方案 §〇 v2):
//!
//! ```json
//! {
//!   "source": "editor",
//!   "nodes": [{"id": 5, "type": "Rectangle", "identifier": "vector_nodes::…::RectangleNode",
//!              "display_name": "", "is_layer": false, "position": [10, 10],
//!              "params": {"1": {"F64": 200.0}}}],
//!   "wires": [{"from": {"node": 7, "output": 0}, "to": {"node": 8, "input": 1}}],
//!   "layers": [{"id": 3, "name": "Layer", "children": []}]
//! }
//! ```

use document_format::{Gdd, GddV1Layout};
use graph_craft::document::{DocumentNodeImplementation, NodeInput, NodeNetwork};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

/// Where the inspected structure came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InspectSource {
	/// The editor's own document view (save-pipeline snapshot).
	Editor,
	/// The compiler-stack registry of a `.gdd` the editor cannot open.
	Registry,
}

/// A node entry in the inspected document.
#[derive(Debug, Clone, Serialize)]
pub struct InspectNode {
	pub id: u64,
	/// Short type name (last path segment of the identifier, e.g. `"RectangleNode"`).
	#[serde(rename = "type")]
	pub node_type: Option<String>,
	/// Full node identifier: the persistent `reference` when present (e.g. `"Merge"`), otherwise
	/// the proto-node path (e.g. `"vector_nodes::generator_nodes::RectangleNode"`).
	pub identifier: Option<String>,
	/// User-chosen display name (empty string means "unset" in the editor's convention).
	pub display_name: Option<String>,
	pub is_layer: bool,
	/// Graph-space grid position `[x, y]`, or `null` when the node sits in a chain/stack or the
	/// source carries no position metadata.
	pub position: Option<[i32; 2]>,
	/// Parameter values keyed by input index: only `NodeInput::Value` inputs appear here.
	/// Node-to-node connections surface as `wires` instead.
	pub params: Map<String, Value>,
}

/// A wire from one node's output to another node's input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InspectWire {
	pub from: WireSource,
	pub to: WireTarget,
}

/// Source endpoint of a wire: producer node + output index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WireSource {
	pub node: u64,
	pub output: usize,
}

/// Target endpoint of a wire: consumer node + input index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WireTarget {
	pub node: u64,
	pub input: usize,
}

/// A layer hierarchy entry (children are nested deeper in the layer tree).
#[derive(Debug, Clone, Serialize)]
pub struct InspectLayer {
	pub id: u64,
	pub name: Option<String>,
	pub children: Vec<InspectLayer>,
}

/// Full inspection result for one document network (sub-graphs are not expanded).
#[derive(Debug, Clone, Serialize)]
pub struct DocumentInspect {
	/// Which read path produced this structure.
	pub source: InspectSource,
	#[serde(rename = "nodeCount")]
	pub node_count: usize,
	pub nodes: Vec<InspectNode>,
	pub wires: Vec<InspectWire>,
	pub layers: Vec<InspectLayer>,
}

// ================================================================================================
// Read path 1: editor save-pipeline snapshot (JSON)
// ================================================================================================

/// Parse the editor's serialized document (as produced by
/// [`crate::session::HeadlessSession::capture_document_json`]).
pub fn inspect_editor_snapshot(serialized: &[u8]) -> Result<DocumentInspect, String> {
	let document: Value = serde_json::from_slice(serialized).map_err(|error| format!("Document snapshot is not valid JSON: {error}"))?;
	inspect_snapshot_value(&document)
}

/// Same as [`inspect_editor_snapshot`] but for an already-parsed snapshot.
pub fn inspect_snapshot_value(document: &Value) -> Result<DocumentInspect, String> {
	let network_interface = &document["network_interface"];
	let network = &network_interface["network"];
	let nodes_value = network["nodes"].as_array().ok_or("Document snapshot has no node table")?;

	// Node metadata: display name, layer flag, reference, and position live in
	// network_metadata.persistent_metadata.node_metadata, keyed by node id.
	let mut display_names = HashMap::<u64, Option<String>>::new();
	let mut layer_flags = HashMap::<u64, bool>::new();
	let mut references = HashMap::<u64, Option<String>>::new();
	let mut positions = HashMap::<u64, Option<[i32; 2]>>::new();
	if let Some(entries) = network_interface["network_metadata"]["persistent_metadata"]["node_metadata"].as_array() {
		for entry in entries {
			let (Some(id), Some(value)) = (entry.get(0).and_then(Value::as_u64), entry.get(1)) else { continue };
			// Values are wrapped as {"persistent_metadata": {…}}; be lenient if unwrapped.
			let persistent = value.get("persistent_metadata").unwrap_or(value);
			display_names.insert(id, persistent["display_name"].as_str().map(str::to_string));
			layer_flags.insert(id, persistent["node_type_metadata"].get("Layer").is_some());
			references.insert(id, persistent["reference"].as_str().map(str::to_string));
			positions.insert(id, absolute_position(&persistent["node_type_metadata"]));
		}
	}

	let mut nodes = Vec::with_capacity(nodes_value.len());
	let mut wires = Vec::new();
	for entry in nodes_value {
		let (Some(id), Some(node)) = (entry.get(0).and_then(Value::as_u64), entry.get(1)) else { continue };
		let inputs = node["inputs"].clone();

		let mut params = Map::new();
		if let Some(inputs_array) = inputs.as_array() {
			for (input_index, input) in inputs_array.iter().enumerate() {
				// Connections: {"Node": {"node_id": …, "output_index": …}} → wire.
				if let Some(connection) = input.get("Node")
					&& let (Some(from_node), Some(from_output)) = (connection["node_id"].as_u64(), connection["output_index"].as_u64())
				{
					wires.push(InspectWire {
						from: WireSource { node: from_node, output: from_output as usize },
						to: WireTarget { node: id, input: input_index },
					});
					continue;
				}
				// Parameters: {"Value": {"tagged_value": …, "exposed": …}} → params[index].
				if let Some(value) = input.get("Value")
					&& let Some(tagged) = value.get("tagged_value")
				{
					params.insert(input_index.to_string(), tagged.clone());
				}
			}
		}

		let identifier = references.get(&id).cloned().flatten().or_else(|| node_reference(node));
		nodes.push(InspectNode {
			id,
			node_type: identifier.as_deref().map(short_type_name),
			identifier,
			display_name: display_names.get(&id).cloned().flatten(),
			is_layer: layer_flags.get(&id).copied().unwrap_or(false),
			position: positions.get(&id).cloned().flatten(),
			params,
		});
	}

	// Stable output order: ascending node id.
	nodes.sort_by_key(|node| node.id);
	wires.sort_by_key(|wire| (wire.to.node, wire.to.input));

	let nodes_by_id: HashMap<u64, &Value> =
		nodes_value.iter().filter_map(|entry| Some((entry.get(0)?.as_u64()?, entry.get(1)?))).collect();
	let layers = derive_layer_tree(network, &nodes_by_id, &display_names, &layer_flags);

	Ok(DocumentInspect {
		source: InspectSource::Editor,
		node_count: nodes.len(),
		nodes,
		wires,
		layers,
	})
}

/// Extract `[x, y]` from a `NodeTypePersistentMetadata` JSON value (`{"Layer": {"position": …}}` /
/// `{"Node": {"position": …}}`); `Absolute([x, y])` yields a position, stack/chain yield `None`.
fn absolute_position(node_type_metadata: &Value) -> Option<[i32; 2]> {
	let variant = node_type_metadata.as_object()?.values().next()?;
	let position = variant.get("position")?;
	let absolute = position.get("Absolute")?;
	let coords = absolute.as_array()?;
	Some([coords.first()?.as_i64()? as i32, coords.get(1)?.as_i64()? as i32])
}

/// A node's type from its serialized form: the proto implementation name
/// (e.g. `vector_nodes::generator_nodes::RectangleNode`).
fn node_reference(node: &Value) -> Option<String> {
	if let Some(reference) = node["reference"].as_str() {
		return Some(reference.to_string());
	}
	node["implementation"]["ProtoNode"]["name"].as_str().map(str::to_string)
}

/// `"a::b::RectangleNode"` → `"RectangleNode"`.
fn short_type_name(identifier: &str) -> String {
	identifier.rsplit("::").next().unwrap_or(identifier).to_string()
}

/// Follow the node-id referenced by `input` (the `{"Node": {…}}` variant), if any.
fn upstream_node(input: &Value) -> Option<u64> {
	Some(input.get("Node")?["node_id"].as_u64()?)
}

/// Walk `start` then repeatedly its primary input (input 0), stopping at cycles.
fn primary_chain(nodes_by_id: &HashMap<u64, &Value>, start: u64, visited: &mut HashSet<u64>) -> Vec<u64> {
	let mut chain = Vec::new();
	let mut current = Some(start);
	while let Some(id) = current {
		if !visited.insert(id) {
			break;
		}
		chain.push(id);
		current = nodes_by_id.get(&id).and_then(|node| node["inputs"].get(0)).and_then(upstream_node);
	}
	chain
}

/// Derive the layer hierarchy: root-level layers from the primary flow chain (root = export 0),
/// children from each layer's horizontal flow (inputs ≥ 1), recursively. Mirrors the editor's
/// `build_layer_structure` rules.
fn derive_layer_tree(
	network: &Value,
	nodes_by_id: &HashMap<u64, &Value>,
	display_names: &HashMap<u64, Option<String>>,
	layer_flags: &HashMap<u64, bool>,
) -> Vec<InspectLayer> {
	let is_layer = |id: u64| layer_flags.get(&id).copied().unwrap_or(false);

	// Root node = export 0 ({"Node": {"node_id": …}}); an empty document has none.
	let Some(root) = network["exports"].get(0).and_then(upstream_node) else {
		return Vec::new();
	};

	// Root-level layers sit on the primary chain from the root, topmost first.
	let mut visited = HashSet::new();
	let root_layers: Vec<u64> = primary_chain(nodes_by_id, root, &mut visited).into_iter().filter(|id| is_layer(*id)).collect();

	let mut ancestors = HashSet::new();
	ancestors.extend(root_layers.iter().copied());

	root_layers
		.iter()
		.map(|id| build_layer_entry(*id, &ancestors, nodes_by_id, display_names, layer_flags))
		.collect()
}

fn build_layer_entry(
	id: u64,
	ancestors: &HashSet<u64>,
	nodes_by_id: &HashMap<u64, &Value>,
	display_names: &HashMap<u64, Option<String>>,
	layer_flags: &HashMap<u64, bool>,
) -> InspectLayer {
	let is_layer = |node: u64| layer_flags.get(&node).copied().unwrap_or(false);

	// Children: layers reachable from this layer's horizontal-flow inputs (inputs ≥ 1), each
	// followed up its primary chain, excluding ancestors to guard against cycles.
	let mut child_ancestors = ancestors.clone();
	child_ancestors.insert(id);

	let horizontal_starts: Vec<u64> = nodes_by_id
		.get(&id)
		.and_then(|node| node["inputs"].as_array())
		.map(|inputs| inputs.iter().skip(1).filter_map(upstream_node).collect())
		.unwrap_or_default();

	let mut children = Vec::new();
	let mut seen = HashSet::new();
	for start in horizontal_starts {
		let mut visited = HashSet::new();
		for candidate in primary_chain(nodes_by_id, start, &mut visited) {
			if is_layer(candidate) && !child_ancestors.contains(&candidate) && seen.insert(candidate) {
				children.push(build_layer_entry(candidate, &child_ancestors, nodes_by_id, display_names, layer_flags));
			}
		}
	}

	InspectLayer { id, name: display_names.get(&id).cloned().flatten(), children }
}

// ================================================================================================
// Read path 2: graph-storage registry fallback (compiler-stack .gdd files)
// ================================================================================================

/// Which read path a `.gdd` file must take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentProbe {
	/// The archive embeds an editor document (legacy blob): the editor can open it.
	EditorBacked,
	/// Compiler-stack registry only (e.g. `graphene-cli new --with-nodes` fixtures): the editor
	/// cannot open these — attempting it panics inside graph flattening
	/// (`graph-craft/src/document.rs:980`), so callers must route to the registry read path or
	/// fail with a clean error instead of touching the editor.
	RegistryOnly,
}

/// Probe a `.gdd` file for an embedded editor document without touching the editor (pure
/// document-format read; cannot panic on compiler-stack documents).
pub async fn probe_gdd_document(path: &std::path::Path) -> Result<DocumentProbe, String> {
	let bytes = std::fs::read(path).map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
	let container = document_container::AnyContainer::Memory(document_container::backends::memory::MemoryBackend::new());
	let gdd = Gdd::open_from_archive(&bytes, container, GddV1Layout)
		.await
		.map_err(|error| format!("Failed to open {}: {error}", path.display()))?;
	if gdd.read_legacy_document().await.is_some() {
		Ok(DocumentProbe::EditorBacked)
	} else {
		Ok(DocumentProbe::RegistryOnly)
	}
}

/// Open a `.gdd` archive and convert its graph-storage registry back into a [`NodeNetwork`] —
/// the read path for editor-unopenable compiler-stack documents (e.g. `graphene-cli new
/// --with-nodes` fixtures). An empty registry (no root network) converts to an empty network.
pub async fn read_registry_network(path: &std::path::Path) -> Result<NodeNetwork, String> {
	use document_graph_storage::to_runtime::ConversionError;
	let bytes = std::fs::read(path).map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
	let container = document_container::AnyContainer::Memory(document_container::backends::memory::MemoryBackend::new());
	let gdd = Gdd::open_from_archive(&bytes, container, GddV1Layout)
		.await
		.map_err(|error| format!("Failed to open {}: {error}", path.display()))?;
	if gdd.registry().networks.is_empty() {
		return Ok(NodeNetwork::default());
	}
	let declarations = gdd.declarations(&gdd).await;
	match gdd.registry().to_runtime_with_metadata(&declarations) {
		Ok((network, _metadata)) => Ok(network),
		// A registry whose root network is absent is, structurally, an empty document.
		Err(ConversionError::NetworkNotFound(_)) => Ok(NodeNetwork::default()),
		Err(error) => Err(format!("Failed to convert the registry of {}: {error}", path.display())),
	}
}

/// Inspect a `NodeNetwork` converted from a `.gdd` registry (the editor cannot open those files,
/// see module docs). Editor-side metadata (positions, layer tree) does not exist in this form.
pub fn inspect_registry_network(network: &NodeNetwork) -> DocumentInspect {
	let mut nodes = Vec::with_capacity(network.nodes.len());
	let mut wires = Vec::new();

	let mut ids: Vec<u64> = network.nodes.keys().map(|id| id.0).collect();
	ids.sort_unstable();

	for id in ids {
		let node_id = graph_craft::document::NodeId(id);
		let node = &network.nodes[&node_id];

		let mut params = Map::new();
		for (input_index, input) in node.inputs.iter().enumerate() {
			match input {
				// Connections → wires.
				NodeInput::Node { node_id: from, output_index } => wires.push(InspectWire {
					from: WireSource { node: from.0, output: *output_index },
					to: WireTarget { node: id, input: input_index },
				}),
				// Parameters → params[index]; `MemoHash` serializes transparently as the value.
				NodeInput::Value { tagged_value, .. } => {
					if let Ok(value) = serde_json::to_value(tagged_value) {
						params.insert(input_index.to_string(), value);
					}
				}
				_ => {}
			}
		}

		let identifier = match &node.implementation {
			DocumentNodeImplementation::ProtoNode(proto) => Some(proto.as_str().to_string()),
			DocumentNodeImplementation::Network(_) => None,
			other => Some(format!("{other:?}")),
		};

		nodes.push(InspectNode {
			id,
			node_type: identifier.as_deref().map(short_type_name),
			identifier,
			display_name: None,
			is_layer: false,
			position: None,
			params,
		});
	}

	wires.sort_by_key(|wire| (wire.to.node, wire.to.input));

	DocumentInspect {
		source: InspectSource::Registry,
		node_count: nodes.len(),
		nodes,
		wires,
		// The compiler-stack registry carries no layer tree.
		layers: Vec::new(),
	}
}
