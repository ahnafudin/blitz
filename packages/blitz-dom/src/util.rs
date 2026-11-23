use crate::node::{Node, NodeData};
use color::{AlphaColor, Srgb};
use keyboard_types::Modifiers;
use std::borrow::Cow;
use style::color::AbsoluteColor;

#[cfg(target_os = "macos")]
pub(crate) const ACTION_MOD: Modifiers = Modifiers::META;
#[cfg(not(target_os = "macos"))]
pub(crate) const ACTION_MOD: Modifiers = Modifiers::CONTROL;

pub type Color = AlphaColor<Srgb>;

/// Decode raw font bytes, decompressing WOFF/WOFF2 if the `woff` feature is enabled.
/// Returns the original slice unchanged for TTF/OTF input, and also on decompression
/// failure. With the `woff` feature disabled, all input passes through unchanged.
pub fn decode_font_bytes(bytes: &[u8]) -> Cow<'_, [u8]> {
    if bytes.len() < 4 {
        return Cow::Borrowed(bytes);
    }
    match &bytes[0..4] {
        #[cfg(feature = "woff")]
        b"wOFF" => wuff::decompress_woff1(bytes)
            .map(Cow::Owned)
            .unwrap_or_else(|_| {
                #[cfg(feature = "tracing")]
                tracing::warn!("Failed to decompress woff1 font");
                Cow::Borrowed(bytes)
            }),
        #[cfg(feature = "woff")]
        b"wOF2" => wuff::decompress_woff2(bytes)
            .map(Cow::Owned)
            .unwrap_or_else(|_| {
                #[cfg(feature = "tracing")]
                tracing::warn!("Failed to decompress woff2 font");
                Cow::Borrowed(bytes)
            }),
        _ => Cow::Borrowed(bytes),
    }
}

#[cfg(feature = "svg")]
use std::sync::{Arc, LazyLock};
#[cfg(feature = "svg")]
use usvg::fontdb;
#[cfg(feature = "svg")]
pub(crate) static FONT_DB: LazyLock<Arc<fontdb::Database>> = LazyLock::new(|| {
    #[cfg_attr(
        all(feature = "ahcut", not(feature = "system-fonts")),
        allow(unused_mut)
    )]
    let mut db = fontdb::Database::new();
    // With `ahcut`, a build without `system-fonts` reads no font of the machine here either.
    #[cfg(any(feature = "system-fonts", not(feature = "ahcut")))]
    db.load_system_fonts();
    Arc::new(db)
});

/// Which kind of CSS image layer list (`background-image` or `mask-image`) to
/// flush from style to dedicated storage on the node.
#[derive(Clone, Copy, Debug)]
pub enum ImageLayerKind {
    Background,
    Mask,
}

impl ImageLayerKind {
    pub fn image_type(self, idx: usize) -> ImageType {
        match self {
            Self::Background => ImageType::Background(idx),
            Self::Mask => ImageType::Mask(idx),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImageType {
    Image,
    Background(usize),
    Mask(usize),
}

/// A point
#[derive(Clone, Debug, Copy, Eq, PartialEq)]
pub struct Point<T> {
    /// The x coordinate
    pub x: T,
    /// The y coordinate
    pub y: T,
}

impl Point<f64> {
    pub const ZERO: Self = Point { x: 0.0, y: 0.0 };
}

// Debug print an RcDom
pub fn walk_tree(indent: usize, node: &Node) {
    // Skip all-whitespace text nodes entirely
    if let NodeData::Text(data) = &node.data {
        if data.content.chars().all(|c| c.is_ascii_whitespace()) {
            return;
        }
    }

    print!("{}", " ".repeat(indent));
    let id = node.id;
    match &node.data {
        NodeData::Document(_) => println!("#Document {id}"),

        NodeData::Text(data) => {
            if data.content.chars().all(|c| c.is_ascii_whitespace()) {
                println!("{id} #text: <whitespace>");
            } else {
                let content = data.content.trim();
                if content.len() > 10 {
                    println!(
                        "#text {id}: {}...",
                        content
                            .split_at(content.char_indices().take(10).last().unwrap().0)
                            .0
                            .escape_default()
                    )
                } else {
                    println!("#text {id}: {}", data.content.trim().escape_default())
                }
            }
        }

        NodeData::Comment { .. } => println!("<!-- COMMENT {id} -->"),

        NodeData::AnonymousBlock(_) => println!("{id} AnonymousBlock"),

        NodeData::Element(data) => {
            print!("<{} {id}", data.name.local);
            for attr in data.attrs.iter() {
                print!(" {}=\"{}\"", attr.name.local, attr.value);
            }
            if !node.children.is_empty() {
                println!(">");
            } else {
                println!("/>");
            }
        } // NodeData::Doctype {
          //     ref name,
          //     ref public_id,
          //     ref system_id,
          // } => println!("<!DOCTYPE {} \"{}\" \"{}\">", name, public_id, system_id),
          // NodeData::ProcessingInstruction { .. } => unreachable!(),
    }

    if !node.children.is_empty() {
        for child_id in node.children.iter() {
            walk_tree(indent + 2, node.with(*child_id));
        }

        if let NodeData::Element(data) = &node.data {
            println!("{}</{}>", " ".repeat(indent), data.name.local);
        }
    }
}

/// Parse an SVG from its serialized `source`.
///
/// Pass `images` (the document URL and image cache) for an inline `<svg>` or
/// the root of an SVG document. The mutator loads the hrefs of its `<image>`
/// and `<feImage>` elements like `<img src>`, so usvg resolves them from the
/// image cache. Images still loading are left out until they arrive and
/// reconstruct the `<svg>`.
///
/// Pass `None` for SVG images (`<img src="x.svg">`, CSS images), which don't
/// load external resources, as browsers render them in "secure animated mode".
///
/// Either way usvg never reads a local file.
#[cfg(feature = "svg")]
pub(crate) fn parse_svg(
    source: &[u8],
    images: Option<(
        &crate::url::DocumentUrl,
        &std::collections::HashMap<String, crate::node::ImageData>,
    )>,
    #[cfg(feature = "ahcut")] budget: Option<&crate::SvgImageBudget>,
) -> Result<crate::node::SvgImageData, usvg::Error> {
    // A checked render loads no external image: the mutator starts no such load and usvg is handed none.
    #[cfg(feature = "ahcut")]
    {
        let _ = images;
        parse_svg_refusing_external(source, budget)
    }
    #[cfg(not(feature = "ahcut"))]
    {
        let options = usvg::Options {
            fontdb: Arc::clone(&*FONT_DB),
            image_href_resolver: usvg::ImageHrefResolver {
                resolve_data: usvg::ImageHrefResolver::default_data_resolver(),
                resolve_string: Box::new(move |href, _| {
                    let (base_url, image_cache) = images?;
                    svg_image_kind(image_cache.get(base_url.resolve_relative(href)?.as_str())?)
                }),
            },
            ..Default::default()
        };
        crate::node::SvgImageData::from_data(source, &options)
    }
}

/// A loaded image as an image usvg can render
#[cfg(all(feature = "svg", not(feature = "ahcut")))]
fn svg_image_kind(image: &crate::node::ImageData) -> Option<usvg::ImageKind> {
    use crate::node::ImageData;
    match image {
        // TODO: hand usvg the decoded pixels instead of re-encoding them once
        // usvg supports that: https://github.com/linebender/resvg/issues/1096
        ImageData::Raster(raster) => {
            encode_png(raster).map(|png| usvg::ImageKind::PNG(Arc::new(png)))
        }
        ImageData::Svg(svg) => Some(usvg::ImageKind::SVG((*svg.tree).clone())),
        ImageData::None => None,
    }
}

#[cfg(all(feature = "svg", not(feature = "ahcut")))]
fn encode_png(image: &crate::node::RasterImageData) -> Option<Vec<u8>> {
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fastest);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(image.data.data()).ok()?;
    writer.finish().ok()?;
    Some(png)
}

/// [`parse_svg`] for a checked render. usvg loads an image that an `<image>` or `<feImage>` names through
/// these two resolvers. A `data:` URL is decoded from memory; an SVG in it is looked at for text and for image
/// sources of its own before usvg parses it, and usvg forwards the `data:` URLs of a nested SVG here too. Any
/// other source is a path or URL that usvg would read from disk: it is recorded and never opened.
#[cfg(feature = "ahcut")]
pub(crate) fn parse_svg_refusing_external(
    source: &[u8],
    budget: Option<&crate::SvgImageBudget>,
) -> Result<crate::node::SvgImageData, usvg::Error> {
    let checked = budget.map(crate::svg_budget::CheckedParse::new);
    let source = match checked.as_ref() {
        Some(check) => check.xml(source)?,
        None => Cow::Borrowed(source),
    };
    let embedded = std::sync::Mutex::new(crate::node::SvgRefusals::default());
    let default_data = usvg::ImageHrefResolver::default_data_resolver();
    let options = usvg::Options {
        fontdb: Arc::clone(&*FONT_DB),
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|mime: &str, data: Arc<Vec<u8>>, options: &usvg::Options| {
                // usvg sniffs text/plain with imagesize and treats other bytes as a nested SVG.
                let raster = match mime {
                    "image/jpeg" | "image/jpg" | "image/png" | "image/gif" | "image/webp" => true,
                    "image/svg+xml" => false,
                    "text/plain" => matches!(
                        imagesize::image_type(&data),
                        Ok(imagesize::ImageType::Jpeg
                            | imagesize::ImageType::Png
                            | imagesize::ImageType::Gif
                            | imagesize::ImageType::Webp)
                    ),
                    _ => return None,
                };
                if let Some(check) = checked.as_ref()
                    && !check.source_fits(&data)
                {
                    return None;
                }
                if raster {
                    if let Some(check) = checked.as_ref() {
                        check.charge_raster(&data)?;
                    }
                    return default_data(mime, data, options);
                }
                let _level = match checked.as_ref() {
                    Some(check) => Some(check.enter_svg()?),
                    None => None,
                };
                let data = if let Some(check) = checked.as_ref() {
                    match check.xml(&data).ok()? {
                        Cow::Borrowed(_) => data,
                        Cow::Owned(bytes) => Arc::new(bytes),
                    }
                } else {
                    data
                };
                if let Some(found) = crate::node::SvgRefusals::of_bytes(&data) {
                    embedded.lock().unwrap().merge(found);
                }
                default_data(mime, data, options)
            }),
            resolve_string: Box::new(|href: &str, _: &usvg::Options| {
                embedded
                    .lock()
                    .unwrap()
                    .merge(crate::node::SvgRefusals::external(href));
                None
            }),
        },
        ..Default::default()
    };
    let mut svg = crate::node::SvgImageData::from_data(&source, &options)?;
    drop(options);
    svg.reservation = checked.map(crate::svg_budget::CheckedParse::into_reservation);
    let embedded = embedded.into_inner().unwrap();
    svg.text_element = svg.text_element.or(embedded.text_element);
    if svg.external_image.is_none() {
        svg.external_image = embedded.external_image;
    }
    Ok(svg)
}

pub trait ToColorColor {
    /// Converts a color into the `AlphaColor<Srgb>` type from the `color` crate
    fn as_color_color(&self) -> Color;
}
impl ToColorColor for AbsoluteColor {
    fn as_color_color(&self) -> Color {
        Color::new(
            *self
                .to_color_space(style::color::ColorSpace::Srgb)
                .raw_components(),
        )
    }
}

#[cfg(all(test, feature = "svg"))]
pub(crate) fn parse_svg_fixture(source: &[u8]) -> Result<crate::node::SvgImageData, usvg::Error> {
    parse_svg(
        source,
        None,
        #[cfg(feature = "ahcut")]
        None,
    )
}

#[cfg(all(test, feature = "svg"))]
mod svg_tests {
    use super::parse_svg_fixture;

    #[test]
    fn svg_without_xmlns_parses() {
        let src = br#"<svg viewBox="0 0 24 24"><path d="M0 0h24v24H0z"/></svg>"#;
        let svg = parse_svg_fixture(src).unwrap();
        assert!(!svg.tree.root().children().is_empty());
    }

    #[test]
    fn missing_height_is_computed_from_width_and_viewbox_ratio() {
        let src = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" width="200"><rect width="100%" height="100%" fill="green"/></svg>"#;
        let svg = parse_svg_fixture(src).unwrap();
        assert_eq!(svg.intrinsic_width(), Some(200.0));
        assert_eq!(svg.intrinsic_height(), None);
        assert_eq!(svg.viewbox_aspect_ratio(), Some(1.0));
        assert_eq!(svg.tree.size().width(), 200.0);
        assert_eq!(svg.intrinsic_size(), (200.0, 200.0));
    }

    #[test]
    fn viewbox_only_has_no_intrinsic_dimensions() {
        let src = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 485 58"></svg>"#;
        let svg = parse_svg_fixture(src).unwrap();
        assert_eq!(svg.intrinsic_width(), None);
        assert_eq!(svg.intrinsic_height(), None);
        // The aspect ratio is still available from the viewBox.
        assert!((svg.aspect_ratio() - (485.0 / 58.0)).abs() < 1e-3);
    }

    #[test]
    fn absolute_dimensions_are_intrinsic() {
        let src = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="16" viewBox="0 0 48 32"></svg>"#;
        let svg = parse_svg_fixture(src).unwrap();
        assert_eq!(svg.intrinsic_width(), Some(24.0));
        assert_eq!(svg.intrinsic_height(), Some(16.0));
    }

    #[test]
    fn percentage_dimensions_are_not_intrinsic() {
        let src = br#"<svg xmlns="http://www.w3.org/2000/svg" width="100%" height="50%" viewBox="0 0 200 100"></svg>"#;
        let svg = parse_svg_fixture(src).unwrap();
        assert_eq!(svg.intrinsic_width(), None);
        assert_eq!(svg.intrinsic_height(), None);
    }

    #[test]
    fn unit_lengths_are_intrinsic() {
        let src = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24px" height="1.5em" viewBox="0 0 48 32"></svg>"#;
        let svg = parse_svg_fixture(src).unwrap();
        assert!(svg.intrinsic_width().is_some());
        assert!(svg.intrinsic_height().is_some());
    }

    #[test]
    fn non_numeric_dimensions_are_not_intrinsic() {
        let src = br#"<svg xmlns="http://www.w3.org/2000/svg" width="auto" height="foo" viewBox="0 0 200 100"></svg>"#;
        let svg = parse_svg_fixture(src).unwrap();
        assert_eq!(svg.intrinsic_width(), None);
        assert_eq!(svg.intrinsic_height(), None);
    }
}

/// Creates an markup5ever::QualName.
/// Given a local name and an optional namespace
#[macro_export]
macro_rules! qual_name {
    ($local:tt $(, $ns:ident)?) => {
        $crate::QualName {
            prefix: None,
            ns: $crate::ns!($($ns)?),
            local: $crate::local_name!($local),
        }
    };
}
