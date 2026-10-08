//! `node-list` primitive: the authoritative, editor-side catalog of document node definitions.
//!
//! Data source: the editor's static `DOCUMENT_NODE_TYPES` table
//! (`editor/src/messages/portfolio/document/node_graph/document_node_definitions.rs:130`), accessed
//! through its public readers — `collect_node_types()` (every definition with a non-empty
//! category, i.e. exactly what the GUI offers) plus `resolve_document_node_type()` for the full
//! [`NodeTemplate`] schema (inputs/defaults/outputs/layer-ness).
//!
//! Zero-document mode: the table is a `once_cell::sync::Lazy` over static data, so the listing
//! needs no editor session and has no side effects — a read-only query primitive in the spirit of
//! `inspect`.
//!
//! Why it exists: `CreateNodeFromContextMenu` takes a full proto path / network identifier and
//! **silently does nothing** on a wrong guess (PY-2 finding: `Blur` lives in
//! `raster_nodes::filter`, not `vector_nodes`). Agents must key node types off this list instead
//! of guessing. The `identifier` field is emitted in the `DefinitionIdentifier` serde form
//! (`{"type":"Network","data":"Rectangle"}`), which can be pasted verbatim into
//! `exec`'s `CreateNodeFromContextMenu` JSON.

use graphite_editor::messages::portfolio::document::node_graph::document_node_definitions::{
    collect_node_types, resolve_document_node_type, DefinitionIdentifier,
};
use graph_craft::document::NodeInput;
use serde::Serialize;

/// One node definition in the catalog.
#[derive(Debug, Clone, Serialize)]
pub struct NodeListEntry {
    /// `DefinitionIdentifier` in its serde form — paste directly into
    /// `CreateNodeFromContextMenu.node_type` via `exec`.
    pub identifier: DefinitionIdentifier,
    /// Stable flat form (`NETWORK:Rectangle` / `PROTONODE:graphene_core::…`) for grep/filtering.
    pub serialized: String,
    pub display_name: String,
    pub category: String,
    /// Whether creating this definition yields a layer (as opposed to a plain node).
    pub is_layer: bool,
    pub inputs: Vec<NodeListInput>,
    /// Output slot names in order.
    pub outputs: Vec<String>,
    /// User-facing description; empty when the editor only has a `TODO` placeholder.
    pub description: String,
}

/// One input slot of a node definition.
#[derive(Debug, Clone, Serialize)]
pub struct NodeListInput {
    pub index: usize,
    /// Input slot name from the definition's input metadata (type name when unnamed).
    pub name: String,
    /// Tooltip description from the definition's input metadata.
    pub description: String,
    /// The default the definition instantiates: `kind` is `value` (a concrete `TaggedValue`
    /// payload, including editor-internal `TypeDefault` placeholders), `import` (an external
    /// connection slot), `node` (a template-internal wire), or `scope`/`reflection`/`inline`.
    pub default: serde_json::Value,
}

/// Collect the full node catalog, optionally filtered by a case-insensitive substring match on
/// display name, identifier (serialized form), and category. Sorted by (category, display name).
pub fn collect(filter: Option<&str>) -> Vec<NodeListEntry> {
    let needle = filter.map(|filter| filter.to_lowercase());
    let mut entries: Vec<NodeListEntry> = collect_node_types()
        .into_iter()
        .filter_map(|frontend| {
            let identifier = DefinitionIdentifier::from_serialized(&frontend.identifier);
            let definition = resolve_document_node_type(&identifier)?;
            let inputs = definition
                .node_template
                .inputs
                .iter()
                .enumerate()
                .map(|(index, input)| NodeListInput {
                    index,
                    name: definition
                        .node_template
                        .input_metadata
                        .get(index)
                        .map(|metadata| {
                            let name = metadata.persistent_metadata.input_name.clone();
                            if name.is_empty() { format!("Input {index}") } else { name }
                        })
                        .unwrap_or_else(|| format!("Input {index}")),
                    description: definition
                        .node_template
                        .input_metadata
                        .get(index)
                        .map(|metadata| metadata.persistent_metadata.input_description.clone())
                        .unwrap_or_default(),
                    default: describe_input(input),
                })
                .collect();

            let entry = NodeListEntry {
                identifier,
                serialized: frontend.identifier.clone(),
                display_name: frontend.name,
                category: definition.category.to_string(),
                is_layer: matches!(
                    definition.node_template.node_type_metadata,
                    graphite_editor::messages::portfolio::document::utility_types::network_interface::NodeTypePersistentMetadata::Layer(_)
                ),
                inputs,
                outputs: definition.node_template.output_names.clone(),
                description: if definition.description == "TODO" { String::new() } else { definition.description.to_string() },
            };

            // Substring filter over display name, serialized identifier, and category.
            match &needle {
                Some(needle) => {
                    let haystack = format!("{} {} {}", entry.display_name, entry.serialized, entry.category).to_lowercase();
                    haystack.contains(needle).then_some(entry)
                }
                None => Some(entry),
            }
        })
        .collect();

    entries.sort_by(|a, b| (&a.category, &a.display_name, &a.serialized).cmp(&(&b.category, &b.display_name, &b.serialized)));
    entries
}

/// Classify one `NodeInput` template slot into the JSON `default` descriptor.
fn describe_input(input: &NodeInput) -> serde_json::Value {
    match input {
        // A concrete default value (or the editor's `TypeDefault` placeholder for
        // "materialize the type's default at runtime"). `MemoHash` serializes transparently.
        NodeInput::Value { tagged_value, exposed } => serde_json::json!({
            "kind": "value",
            "data": serde_json::to_value(tagged_value).unwrap_or(serde_json::Value::Null),
            "exposed": exposed,
        }),
        // An external connection slot fed by the parent network.
        NodeInput::Import { import_type: _, import_index } => serde_json::json!({ "kind": "import", "import_index": import_index }),
        // A wire inside the definition's template network (not user-facing).
        NodeInput::Node { node_id, output_index } => {
            serde_json::json!({ "kind": "node", "node_id": node_id.0, "output_index": output_index })
        }
        NodeInput::Scope(key) => serde_json::json!({ "kind": "scope", "key": key.as_ref() }),
        NodeInput::Reflection(_) => serde_json::json!({ "kind": "reflection" }),
        NodeInput::Inline(_) => serde_json::json!({ "kind": "inline" }),
    }
}
