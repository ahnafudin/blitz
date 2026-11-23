//! The checked SVG resolver on small in-memory images and deliberately small cache allowances.

use super::{CACHE, NESTING, PIXELS, SOURCE};
use crate::util::parse_svg_refusing_external;
use crate::{SvgImageBudget, SvgImageLimits};

fn budget(source_bytes: u64, raster_pixels: u64, remaining: u64) -> SvgImageBudget {
    SvgImageBudget::new(
        SvgImageLimits {
            source_bytes,
            raster_pixels,
            raster_edge: 32_768,
            nesting: 2,
        },
        remaining,
    )
}

fn image(href: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><image href="{href}" width="4" height="4"/></svg>"#
    )
}

fn nested(svg: &str) -> String {
    image(&format!(
        "data:image/svg+xml,{}",
        percent_encoding::utf8_percent_encode(svg, percent_encoding::NON_ALPHANUMERIC)
    ))
}

// Transparent 1 x 1 PNG. The source fits the containing file. Decoding uses four RGBA bytes.
const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j6B8AAAAASUVORK5CYII=";

#[test]
fn the_root_and_one_embedded_svg_are_admitted_and_a_third_document_is_refused() {
    let limits = budget(1 << 16, 8, 1 << 18);
    let leaf = image(PNG);
    let _allowed = parse_svg_refusing_external(nested(&leaf).as_bytes(), Some(&limits)).unwrap();
    assert_eq!(limits.refusal(), None);
    let _ = parse_svg_refusing_external(nested(&nested(&leaf)).as_bytes(), Some(&limits));
    assert_eq!(limits.refusal().as_deref(), Some(NESTING));
}

#[test]
fn raster_pixels_are_checked_before_paint() {
    let limits = budget(4096, 0, 4096);
    let _ = parse_svg_refusing_external(image(PNG).as_bytes(), Some(&limits));
    assert_eq!(limits.refusal().as_deref(), Some(PIXELS));
}

#[test]
fn a_raster_that_fits_the_area_can_exceed_the_edge() {
    let mut limits = budget(4096, 1, 4);
    limits.limits.raster_edge = 0;
    let _ = parse_svg_refusing_external(image(PNG).as_bytes(), Some(&limits));
    assert_eq!(
        limits.refusal().as_deref(),
        Some("LIMIT_EXCEEDED:limit.image.edge: resize the image to at most 0 pixels on each edge")
    );
}

#[test]
fn text_plain_sniffs_the_same_raster_as_usvg() {
    let plain = PNG.replace("image/png", "text/plain");
    let limits = budget(4096, 1, 4);
    let tree = parse_svg_refusing_external(image(&plain).as_bytes(), Some(&limits)).unwrap();
    assert_eq!(limits.refusal(), None);
    assert_eq!(limits.available(), 0);
    drop(tree);
    assert_eq!(limits.available(), 4);
}

#[test]
fn image_trees_share_the_allowance_and_clones_hold_their_reservation() {
    let limits = budget(4096, 1, 4);
    let first = parse_svg_refusing_external(image(PNG).as_bytes(), Some(&limits)).unwrap();
    assert_eq!(limits.refusal(), None);
    let other = first.clone();
    drop(first);
    assert_eq!(limits.available(), 0);
    drop(other);
    assert_eq!(limits.available(), 4);
    let held = parse_svg_refusing_external(image(PNG).as_bytes(), Some(&limits)).unwrap();
    let _ = parse_svg_refusing_external(image(PNG).as_bytes(), Some(&limits));
    assert_eq!(limits.refusal().as_deref(), Some(CACHE));
    drop(held);
}

#[test]
fn source_bytes_have_their_own_limit() {
    let source = image(PNG);
    let limits = budget(source.len() as u64 - 1, 1, 4096);
    let _ = parse_svg_refusing_external(source.as_bytes(), Some(&limits));
    assert_eq!(limits.refusal().as_deref(), Some(SOURCE));
}

#[test]
fn siblings_leave_their_nesting_level_and_share_their_cache() {
    let svg = nested(&image(PNG));
    let child = svg
        .split_once('>')
        .unwrap()
        .1
        .strip_suffix("</svg>")
        .unwrap();
    let siblings = svg.replace("</svg>", &format!("{child}</svg>"));
    let limits = budget(8192, 1, 8192);
    let tree = parse_svg_refusing_external(siblings.as_bytes(), Some(&limits)).unwrap();
    assert_eq!(limits.refusal(), None);
    assert_eq!(limits.available(), 8192 - 2 * 4);
    drop(tree);
    assert_eq!(limits.available(), 8192);
}

#[test]
fn compressed_xml_is_bounded_by_its_expanded_length() {
    use std::io::Write;
    let xml = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><!--{}--></svg>"#,
        " ".repeat(300)
    );
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(xml.as_bytes()).unwrap();
    let compressed = encoder.finish().unwrap();
    assert!(compressed.len() < xml.len() - 1);
    let limits = budget(compressed.len() as u64 - 1, 1, 4096);
    assert!(parse_svg_refusing_external(&compressed, Some(&limits)).is_err());
    assert_eq!(limits.refusal().as_deref(), Some(SOURCE));

    let limits = budget(xml.len() as u64, 1, xml.len() as u64 - 1);
    assert!(parse_svg_refusing_external(&compressed, Some(&limits)).is_err());
    assert_eq!(limits.refusal().as_deref(), Some(CACHE));
    let limits = budget(compressed.len() as u64, 1, xml.len() as u64);
    let parsed = parse_svg_refusing_external(&compressed, Some(&limits)).unwrap();
    assert_eq!(limits.refusal(), None);
    drop(parsed);
    assert_eq!(limits.available(), xml.len() as u64);
}
