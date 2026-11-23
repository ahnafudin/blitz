//! Where a document draws an SVG that holds what a checked render refuses (text elements, or an image source that is not a `data:` URL): an inline `<svg>`, an image element or an image layer.

use crate::node::{ImageData, SvgImageData};
use crate::{BaseDocument, NodeId};

/// How a node comes to draw the SVG.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SvgTextPlace {
    /// An `<svg>` element in the document.
    InlineSvg,
    /// An element whose image is an SVG, such as `<img>`.
    Image,
    /// A background or mask image layer.
    ImageLayer,
}

/// A node that draws an SVG with a text element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SvgTextUse {
    pub node: NodeId,
    /// The first text element of that SVG: `text`, `tspan`, `textPath` or `tref`.
    pub element: &'static str,
    pub place: SvgTextPlace,
}

/// A node that draws an SVG whose `<image>` or `<feImage>` names a source other than a `data:` URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SvgImageUse {
    pub node: NodeId,
    /// `image` or `feImage`.
    pub element: &'static str,
    /// The source as written, cut to 160 characters.
    pub href: String,
    pub place: SvgTextPlace,
}

impl BaseDocument {
    /// A checked image failed its document's limits. This includes a failed parse with no drawable tree.
    pub fn svg_limit_refusal(&self) -> Option<std::borrow::Cow<'static, str>> {
        self.svg_image_budget.as_ref()?.refusal()
    }

    /// The checked painter's FilterError carries static text. The document retains the full remedy.
    pub fn svg_limit_summary(&self) -> Option<&'static str> {
        self.svg_image_budget.as_ref()?.summary()
    }

    /// The first node that holds a parsed SVG for which `pick` finds something, with where it draws it. Every
    /// node is looked at, attached or not, so a frame that passes holds no such SVG at all.
    fn first_svg<T>(
        &self,
        pick: impl Fn(&SvgImageData) -> Option<T>,
    ) -> Option<(NodeId, SvgTextPlace, T)> {
        for (node, data) in self.tree().iter() {
            let Some(element) = data.element_data() else {
                continue;
            };
            if let Some(ImageData::Svg(svg)) = element.image_data()
                && let Some(found) = pick(svg)
            {
                let place = if &*element.name.local == "svg" {
                    SvgTextPlace::InlineSvg
                } else {
                    SvgTextPlace::Image
                };
                return Some((node, place, found));
            }
            for layer in element
                .background_images
                .iter()
                .chain(element.mask_images.iter())
                .flatten()
            {
                if let ImageData::Svg(svg) = &layer.image
                    && let Some(found) = pick(svg)
                {
                    return Some((node, SvgTextPlace::ImageLayer, found));
                }
            }
        }
        None
    }

    /// The first node that holds a parsed SVG with a text element.
    pub fn first_svg_text(&self) -> Option<SvgTextUse> {
        self.first_svg(|svg| svg.text_element)
            .map(|(node, place, element)| SvgTextUse {
                node,
                element,
                place,
            })
    }

    /// The first node that holds a parsed SVG with an image source that is not a `data:` URL.
    pub fn first_svg_external_image(&self) -> Option<SvgImageUse> {
        self.first_svg(|svg| svg.external_image.clone())
            .map(|(node, place, image)| SvgImageUse {
                node,
                element: image.element,
                href: image.href,
                place,
            })
    }
}
