//! The checked SVG resolver on small in-memory images and deliberately small cache allowances.

use crate::util::parse_svg_refusing_external;
use crate::{SvgImageBudget, SvgImageLimits};
use super::{CACHE, NESTING, PIXELS, SOURCE};

fn budget(source_bytes: u64, raster_pixels: u64, remaining: u64) -> SvgImageBudget {
    SvgImageBudget::new(SvgImageLimits { source_bytes, raster_pixels, nesting: 2 }, remaining)
}

fn image(href: &str) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><image href="{href}" width="4" height="4"/></svg>"#)
}

fn nested(svg: &str) -> String {
    image(&format!("data:image/svg+xml,{}", percent_encoding::utf8_percent_encode(svg, percent_encoding::NON_ALPHANUMERIC)))
}

// Transparent 1 x 1 PNG. Raster accounting includes the 68 source bytes and four RGBA bytes.
const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j6B8AAAAASUVORK5CYII=";

#[test]
fn two_embedded_svg_levels_are_admitted_and_a_third_is_refused() {
    let limits=budget(1<<16, 8, 1<<18);
    let leaf = image(PNG);
    let _allowed=parse_svg_refusing_external(nested(&nested(&leaf)).as_bytes(), Some(&limits)).unwrap();
    assert_eq!(limits.refusal(),None);
    let _=parse_svg_refusing_external(nested(&nested(&nested(&leaf))).as_bytes(), Some(&limits));
    assert_eq!(limits.refusal(),Some(NESTING));
}

#[test]
fn raster_pixels_are_checked_before_paint() {
    let limits=budget(4096,0,4096);
    let _=parse_svg_refusing_external(image(PNG).as_bytes(),Some(&limits));
    assert_eq!(limits.refusal(),Some(PIXELS));
}

#[test]
fn image_trees_share_the_allowance_and_clones_hold_their_reservation() {
    let limits=budget(4096,1,72);
    let first=parse_svg_refusing_external(image(PNG).as_bytes(),Some(&limits)).unwrap();
    assert_eq!(limits.refusal(),None);
    let other=first.clone();
    drop(first);
    assert_eq!(limits.available(),0);
    drop(other);
    assert_eq!(limits.available(),72);
    let held=parse_svg_refusing_external(image(PNG).as_bytes(),Some(&limits)).unwrap();
    let _=parse_svg_refusing_external(image(PNG).as_bytes(),Some(&limits));
    assert_eq!(limits.refusal(),Some(CACHE));
    drop(held);
}

#[test]
fn source_bytes_have_their_own_limit() {
    let source=image(PNG);
    let limits=budget(source.len() as u64 - 1,1,4096);
    let _=parse_svg_refusing_external(source.as_bytes(),Some(&limits));
    assert_eq!(limits.refusal(),Some(SOURCE));
}
