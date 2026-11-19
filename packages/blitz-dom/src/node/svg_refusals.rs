//! What a checked render refuses inside an SVG document: text elements, and image sources other than `data:`
//! URLs, including those of an SVG it embeds. Present only with the `ahcut` feature.

use usvg::roxmltree;

/// An image source inside SVG that is not a `data:` URL: a file path, a UNC path, a `file:`, `http:` or
/// `https:` URL, a relative path, or a `data:` URL that usvg could not decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SvgExternalImage {
    /// `image` or `feImage`.
    pub element: &'static str,
    /// The `href` as written, cut to 160 characters.
    pub href: String,
}

impl SvgExternalImage {
    pub(crate) fn new(element: &'static str, href: &str) -> Self {
        Self {
            element,
            href: href.chars().take(160).collect(),
        }
    }
}

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";

/// usvg's rule for an SVG element (`parse_tag_name`): the SVG namespace or none, matched by exact name.
fn is_svg_element(node: &roxmltree::Node) -> bool {
    node.is_element() && matches!(node.tag_name().namespace(), None | Some(SVG_NS))
}

/// Whether usvg takes `href` as a `data:` URL: after leading C0 controls and spaces, `data:` in any case.
/// One it then fails to decode goes to the path resolver, which refuses it.
fn is_data_url(href: &str) -> bool {
    let href = href.trim_start_matches(|c: char| c <= ' ');
    href.get(..5)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("data:"))
}

/// What a checked render refuses in an SVG document, found in its XML.
#[derive(Debug, Clone, Default)]
pub(crate) struct SvgRefusals {
    pub(crate) text_element: Option<&'static str>,
    pub(crate) external_image: Option<SvgExternalImage>,
}

impl SvgRefusals {
    pub(crate) fn of(doc: &roxmltree::Document) -> Self {
        Self {
            text_element: first_text_element(doc),
            external_image: first_external_image(doc),
        }
    }

    /// The same for the bytes of an SVG that usvg loads as a nested image, decoded as usvg decodes them.
    pub(crate) fn of_bytes(data: &[u8]) -> Option<Self> {
        let decompressed;
        let data = if data.starts_with(&[0x1f, 0x8b]) {
            decompressed = usvg::decompress_svgz(data).ok()?;
            decompressed.as_slice()
        } else {
            data
        };
        let xml_options = roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        };
        let text = std::str::from_utf8(data).ok()?;
        let doc = roxmltree::Document::parse_with_options(text, xml_options).ok()?;
        Some(Self::of(&doc))
    }

    /// An href that reached usvg's path resolver.
    pub(crate) fn external(href: &str) -> Self {
        Self {
            text_element: None,
            external_image: Some(SvgExternalImage::new("image", href)),
        }
    }

    /// Keeps the first finding of each kind.
    pub(crate) fn merge(&mut self, other: Self) {
        self.text_element = self.text_element.or(other.text_element);
        if self.external_image.is_none() {
            self.external_image = other.external_image;
        }
    }
}

/// The first element usvg would turn into text. Captured from the XML, because the tree no longer says
/// whether a text element was there when no font matched it. An inline `<svg>` written without `xmlns`
/// reaches the parser without a namespace and is read as SVG all the same.
fn first_text_element(doc: &roxmltree::Document) -> Option<&'static str> {
    doc.descendants().filter(is_svg_element).find_map(|node| {
        ["text", "tspan", "textPath", "tref"]
            .into_iter()
            .find(|name| *name == node.tag_name().name())
    })
}

/// The first `<image>` or `<feImage>` with an `href` (plain or `xlink:`) that is neither a `data:` URL nor a
/// reference to an element of the same document (`#id`).
fn first_external_image(doc: &roxmltree::Document) -> Option<SvgExternalImage> {
    doc.descendants().filter(is_svg_element).find_map(|node| {
        let element = ["image", "feImage"]
            .into_iter()
            .find(|name| *name == node.tag_name().name())?;
        node.attributes()
            .filter(|a| a.name() == "href" && matches!(a.namespace(), None | Some(XLINK_NS)))
            .map(|a| a.value())
            .find(|href| !is_data_url(href) && !href.trim_start().starts_with('#'))
            .map(|href| SvgExternalImage::new(element, href))
    })
}
