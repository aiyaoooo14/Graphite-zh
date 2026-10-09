//! Acceptance tests for the `node-list` enum-variant catalog (CLI 接口方案 反馈清单 §2).
//!
//! The catalog's `enum_variants` entries claim to be the **full legal-value set** of a choice
//! enum input. These tests prove that claim at the serde boundary: every listed variant name
//! must deserialize as a `TaggedValue` of the input's default variant tag (the exact shape an
//! agent pastes into `SetInputValue`), and must round-trip back to the same name.
//!
//! Pure static-data + serde tests: no editor session, no wgpu, thread-safe.

use graphite_cli::node_list::{self, NodeListEntry};

fn collect_all() -> Vec<NodeListEntry> {
    node_list::collect(None)
}

/// Every `enum_variants` entry must be serde-valid for its input's `TaggedValue` tag, and the
/// default value itself must be a member of the list.
#[test]
fn listed_enum_variants_are_serde_legal_values() {
    let entries = collect_all();
    assert!(!entries.is_empty(), "the node catalog must not be empty");

    let mut checked = 0usize;
    let mut checked_inputs = std::collections::BTreeSet::new();

    for entry in &entries {
        for input in &entry.inputs {
            let Some(variants) = &input.enum_variants else { continue };
            assert!(!variants.is_empty(), "{} input {}: empty variant list", entry.serialized, input.index);

            // The default must be a single-key `TaggedValue` object whose key is the enum tag
            // (e.g. `{"BooleanOperation":"Union"}`); that key is what we test the names against.
            let default_data = input
                .default
                .get("data")
                .unwrap_or_else(|| panic!("{} input {}: default must carry data", entry.serialized, input.index));
            let tag = default_data
                .as_object()
                .and_then(|object| object.keys().next())
                .unwrap_or_else(|| panic!("{} input {}: enum-typed default must be a single-key TaggedValue object", entry.serialized, input.index))
                .clone();

            // Distinct names within one list.
            let mut names = variants.iter().map(|variant| variant.name.as_str()).collect::<Vec<_>>();
            let before = names.len();
            names.sort_unstable();
            names.dedup();
            assert_eq!(before, names.len(), "{} input {}: duplicate variant names", entry.serialized, input.index);

            for variant in variants {
                let value = serde_json::json!({ tag.clone(): variant.name });
                let tagged: graph_craft::document::value::TaggedValue = serde_json::from_value(value.clone())
                    .unwrap_or_else(|error| panic!("{} input {}: variant `{}` must deserialize as `{tag}`: {error}", entry.serialized, input.index, variant.name));
                // Round-trip: the accepted spelling must also be the canonical serialized form.
                let back = serde_json::to_value(&tagged).expect("tagged value serializes");
                assert_eq!(back, value, "{} input {}: `{tag}` round-trip mismatch", entry.serialized, input.index);
                checked += 1;
                checked_inputs.insert(tag.clone());
            }

            // The input's own default must be one of the listed variants.
            let default_name = default_data.get(&tag).and_then(|value| value.as_str()).expect("enum default payload is a string");
            assert!(
                variants.iter().any(|variant| variant.name == default_name),
                "{} input {}: default `{default_name}` missing from the listed variants",
                entry.serialized,
                input.index
            );
        }
    }

    assert!(checked > 100, "expected a substantial variant catalog, got {checked}");
    assert!(
        checked_inputs.len() >= 20,
        "expected the catalog to cover most choice enums, got {}: {checked_inputs:?}",
        checked_inputs.len()
    );
}

/// Consistency: the same `TaggedValue` enum tag must resolve to variants everywhere it appears
/// (the mapping is keyed by value variant, not by node).
#[test]
fn enum_variant_lists_are_consistent_across_nodes() {
    let mut per_tag: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    for entry in collect_all() {
        for input in entry.inputs {
            let Some(variants) = input.enum_variants else { continue };
            let tag = input
                .default
                .get("data")
                .and_then(|data| data.as_object().and_then(|object| object.keys().next()))
                .expect("enum-typed input carries a single-key default")
                .clone();
            let names: Vec<String> = variants.into_iter().map(|variant| variant.name).collect();
            match per_tag.entry(tag) {
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(names);
                }
                std::collections::btree_map::Entry::Occupied(slot) => {
                    assert_eq!(slot.get(), &names, "tag `{}` has inconsistent variant lists", slot.key());
                }
            }
        }
    }
}

/// The task's headline cases: BooleanOperation (full 5-variant set, non-`Union` values legal)
/// and the gradient/paint-family enums.
#[test]
fn headline_enums_have_expected_variant_sets() {
    let entries = collect_all();

    // BooleanOperation: the full five-way set (feedback item #2's motivating example).
    let boolean = entries
        .iter()
        .find(|entry| entry.serialized == "PROTONODE:path_bool_nodes::BooleanOperationNode")
        .expect("Boolean Operation node in the catalog");
    let operation = boolean
        .inputs
        .iter()
        .find(|input| input.name == "Operation")
        .expect("Operation input");
    let names: Vec<&str> = operation.enum_variants.as_ref().expect("BooleanOperation variants listed").iter().map(|variant| variant.name.as_str()).collect();
    assert_eq!(names, ["Union", "SubtractFront", "SubtractBack", "Intersect", "Difference"]);

    // Gradient-form family (the `Fill`/`Gradient` parameter space is a color/gradient *value*,
    // not an enum — its enum sub-parameters are these).
    let gradient_form = find_variants_gradient_form();
    assert!(gradient_form.contains(&"Linear".to_string()) && gradient_form.contains(&"Radial".to_string()), "GradientForm variants: {gradient_form:?}");
}

fn find_variants_gradient_form() -> Vec<String> {
    for entry in collect_all() {
        for input in &entry.inputs {
            let Some(variants) = &input.enum_variants else { continue };
            let Some(tag) = input.default.get("data").and_then(|data| data.as_object()).and_then(|object| object.keys().next()) else {
                continue;
            };
            if tag == "GradientForm" {
                return variants.iter().map(|variant| variant.name.clone()).collect();
            }
        }
    }
    panic!("no GradientForm input found in the catalog");
}

/// Hand-maintained fallbacks: BlendMode (29 = 26 grouped + 3 alpha-utility) and ReferencePoint (10).
#[test]
fn fallback_enums_list_full_sets() {
    let mut blend_mode = None;
    let mut reference_point = None;
    for entry in collect_all() {
        for input in &entry.inputs {
            let Some(variants) = &input.enum_variants else { continue };
            let Some(tag) = input.default.get("data").and_then(|data| data.as_object()).and_then(|object| object.keys().next()) else {
                continue;
            };
            match tag.as_str() {
                "BlendMode" => blend_mode = Some(variants.iter().map(|variant| variant.name.clone()).collect::<Vec<_>>()),
                "ReferencePoint" => reference_point = Some(variants.iter().map(|variant| variant.name.clone()).collect::<Vec<_>>()),
                _ => {}
            }
        }
    }

    let blend_mode = blend_mode.expect("BlendMode appears on at least one node input");
    assert_eq!(blend_mode.len(), 29, "BlendMode = 26 grouped + Erase/Restore/MultiplyAlpha: {blend_mode:?}");
    for utility in ["Erase", "Restore", "MultiplyAlpha"] {
        assert!(blend_mode.contains(&utility.to_string()), "alpha-utility mode `{utility}` must be listed");
    }

    let reference_point = reference_point.expect("ReferencePoint appears on at least one node input");
    assert_eq!(
        reference_point,
        ["None", "TopLeft", "TopCenter", "TopRight", "CenterLeft", "Center", "CenterRight", "BottomLeft", "BottomCenter", "BottomRight"]
    );
}
