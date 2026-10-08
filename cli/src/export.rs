//! `export` primitive: run the editor's own export pipeline
//! (`PortfolioMessage::SubmitDocumentExport` → node graph evaluation → encoded file) and write
//! the intercepted `TriggerSaveFile` payload to disk.
//!
//! Format selection follows the output file extension: `.svg` → `FileType::Svg` (the node graph
//! renders SVG segments; the editor hands over the finished SVG text). `.png` → `FileType::Png`
//! (raster export: the node graph renders through wgpu into an RGBA buffer, the editor encodes
//! the PNG — headless-friendly, no frontend rasterization involved).
//!
//! `--width`/`--height` are honored through a two-step export: the editor only accepts a scale
//! factor (resolution = artwork bounds × scale), so the first pass establishes the base size and
//! the second pass submits the corrected scale. `--transparent` is accepted for CLI-compatibility
//! with the L1 tool; PNG exports are always RGBA (transparent background) in this editor chain.
//!
//! If the editor ever answers a raster request with `TriggerExportImage` (an SVG handed to the
//! frontend for rasterization), the command fails loudly: headless mode has no such frontend.

use crate::session::HeadlessSession;
use graphite_editor::messages::frontend::utility_types::{ExportBounds, FileType};
use graphite_editor::messages::prelude::*;
use std::path::{Path, PathBuf};

/// Summary of a finished export (printed as one JSON line).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExportSummary {
	pub saved_path: String,
	pub bytes: usize,
	/// `"svg"` or `"png"`.
	pub format: String,
	/// Decoded pixel size for PNG; parsed from the root element for SVG.
	pub width: Option<u32>,
	pub height: Option<u32>,
	/// Actual scale factor submitted to the editor (differs from `--scale` when
	/// `--width`/`--height` required a corrective pass).
	pub scale: f64,
}

/// Default headless viewport size provided before exporting (the editor's overlay/render update
/// path expects a viewport to exist; `editor/src/test_utils.rs:304-313` sets one the same way).
const VIEWPORT_WIDTH: f64 = 1280.;
const VIEWPORT_HEIGHT: f64 = 720.;

/// Export the document at `document` to `output` through the editor chain.
#[allow(clippy::too_many_arguments)]
pub async fn run(
	session: &mut HeadlessSession,
	document: &Path,
	output: &Path,
	width: Option<u32>,
	height: Option<u32>,
	scale: f64,
	_transparent: bool,
) -> Result<ExportSummary, String> {
	let file_type = detect_file_type(output)?;
	let format = match file_type {
		FileType::Svg => "svg",
		FileType::Png => "png",
		FileType::Jpg => "jpg",
	}
	.to_string();

	session.open_document(document).await?;

	// Headless rendering precondition: give the editor a viewport so overlay/render bookkeeping
	// has a size to work with (same approach as editor test utilities).
	session
		.execute(ViewportMessage::Update { x: 0., y: 0., width: VIEWPORT_WIDTH, height: VIEWPORT_HEIGHT, scale: 1. })
		.await?;

	// Base pass: establishes the artwork's natural size for --width/--height correction.
	let base_scale = if scale > 0. { scale } else { 1.0 };
	let content = submit_export(session, file_type, base_scale).await?;

	let (content, actual_scale) = if width.is_some() || height.is_some() {
		// Measure the base pass, derive the scale that hits the requested size, re-export.
		let (base_width, base_height) = measure(&content, file_type)?;
		let corrective = match (width, height) {
			(Some(w), _) if base_width > 0 => w as f64 / base_width as f64,
			(_, Some(h)) if base_height > 0 => h as f64 / base_height as f64,
			_ => return Err("Cannot derive an export scale from the base pass (zero-sized artwork)".to_string()),
		};
		let corrected = (corrective * 1000.).round() / 1000.;
		if corrected <= 0. {
			return Err("Export scale derived from --width/--height must be positive".to_string());
		}
		let content = submit_export(session, file_type, corrected).await?;
		(content, corrected)
	} else {
		(content, base_scale)
	};

	// Post-conditions: the editor must hand us a fully encoded file, not a rasterization request.
	let images = session.take_captured_export_images();
	if !images.is_empty() {
		return Err(format!(
			"The editor asked the frontend to rasterize an export image ({} request(s)); headless mode cannot rasterize SVG. \
			 This means the node graph produced SVG output for a raster request — please report this.",
			images.len()
		));
	}

	if let Some(parent) = output.parent()
		&& !parent.as_os_str().is_empty()
	{
		std::fs::create_dir_all(parent).map_err(|error| format!("Failed to create directory {}: {error}", parent.display()))?;
	}
	std::fs::write(output, &content).map_err(|error| format!("Failed to write {}: {error}", output.display()))?;

	let (decoded_width, decoded_height) = measure(&content, file_type)?;

	Ok(ExportSummary {
		saved_path: output.display().to_string(),
		bytes: content.len(),
		width: Some(decoded_width),
		height: Some(decoded_height),
		format,
		scale: actual_scale,
	})
}

/// Detect SVG vs PNG from the output extension; anything else is rejected.
fn detect_file_type(output: &Path) -> Result<FileType, String> {
	let extension = output
		.extension()
		.and_then(|extension| extension.to_str())
		.map(|extension| extension.to_ascii_lowercase())
		.ok_or_else(|| format!("Output path {} has no extension (expected .svg or .png)", output.display()))?;
	match extension.as_str() {
		"svg" => Ok(FileType::Svg),
		"png" => Ok(FileType::Png),
		other => Err(format!("Unsupported export extension .{other} (the editor-chain export supports .svg and .png)")),
	}
}

/// Submit one export pass through the editor chain and return the encoded artifact bytes.
async fn submit_export(session: &mut HeadlessSession, file_type: FileType, scale_factor: f64) -> Result<Vec<u8>, String> {
	session.clear_tracking();

	// The editor derives the file name from this + the file type; the authoritative destination
	// is the caller's `-o` path.
	let name = "export".to_string();

	session.dispatch(PortfolioMessage::SubmitDocumentExport {
		name,
		file_type,
		scale_factor,
		bounds: ExportBounds::AllArtwork,
		artboard_name: None,
		artboard_count: 1,
	});
	session.settle().await?;

	if !session.dialogs_shown.is_empty() {
		return Err(format!("Editor raised dialogs during export: {:?}", session.dialogs_shown));
	}

	let mut files = session.take_captured_files();
	let file = files
		.pop()
		.ok_or_else(|| "Export produced no file content — the editor's export pipeline never reached quiescence".to_string())?;
	Ok(file.content)
}

/// Measure the artifact: PNG via the image crate (also validates decodability), SVG from the
/// root element's width/height attributes.
fn measure(content: &[u8], file_type: FileType) -> Result<(u32, u32), String> {
	match file_type {
		FileType::Png => {
			let reader = image::ImageReader::new(std::io::Cursor::new(content))
				.with_guessed_format()
				.map_err(|error| format!("Exported PNG is unreadable: {error}"))?;
			let dimensions = reader.into_dimensions().map_err(|error| format!("Exported PNG cannot be decoded: {error}"))?;
			Ok(dimensions)
		}
		FileType::Svg => {
			let text = std::str::from_utf8(content).map_err(|_| "Exported SVG is not valid UTF-8".to_string())?;
			let width = svg_root_attribute(text, "width").ok_or("Exported SVG root has no width attribute")?;
			let height = svg_root_attribute(text, "height").ok_or("Exported SVG root has no height attribute")?;
			Ok((width, height))
		}
		FileType::Jpg => Err("JPG is not supported by the editor-chain export".to_string()),
	}
}

/// Parse an attribute off the `<svg …>` root element (first `<svg` tag in the document).
fn svg_root_attribute(svg: &str, attribute: &str) -> Option<u32> {
	let start = svg.find("<svg")?;
	let tag_end = svg[start..].find('>')? + start;
	let tag = &svg[start..tag_end];
	let pattern = format!("{attribute}=\"");
	let attribute_start = tag.find(&pattern)? + pattern.len();
	let attribute_end = tag[attribute_start..].find('"')? + attribute_start;
	let raw = &tag[attribute_start..attribute_end];
	// Attributes may carry units ("1280pt"); strip to the leading number.
	let numeric: String = raw.chars().take_while(|character| character.is_ascii_digit() || *character == '.').collect();
	numeric.parse::<f64>().ok().map(|value| value.round() as u32)
}

/// Convenience: re-exported for `main.rs` output formatting.
pub fn print_summary(summary: &ExportSummary) {
	let line = serde_json::json!({
		"saved_path": summary.saved_path,
		"bytes": summary.bytes,
		"format": summary.format,
		"width": summary.width,
		"height": summary.height,
		"scale": summary.scale,
	});
	println!("{line}");
}

/// The output path type used by `main.rs` (kept here so the subcommand stays declarative).
pub type OutputPath = PathBuf;
