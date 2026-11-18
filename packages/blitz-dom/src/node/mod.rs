#![allow(clippy::module_inception)]

mod attributes;
#[cfg(feature = "custom-widget")]
mod custom_widget;
mod element;
mod node;
pub(crate) mod scrollbar;
mod serialize;
mod stylo_data;
#[cfg(feature = "svg")]
mod svg;
#[cfg(feature = "ahcut")]
mod svg_refusals;
mod text;

pub use attributes::{Attribute, Attributes};
#[cfg(feature = "custom-widget")]
pub use custom_widget::{
    ComputedStyles, CustomWidgetData, CustomWidgetStatus, ProxyRenderContext, Widget,
};
pub(crate) use element::parse_classes;
pub use element::{
    CanvasData, DocumentData, ElementData, ImageData, ImageResourceData, LayoutData,
    ListItemLayout, ListItemLayoutPosition, Marker, RasterImageData, SpecialElementData,
    SpecialElementType, Status,
};
pub use node::*;
pub use scrollbar::{ScrollbarColor, ScrollbarRef, ScrollbarWidth};
pub use stylo_data::ComputedStyleRef;
#[cfg(feature = "svg")]
pub use svg::{SvgImageData, SvgIntrinsicDimensions};
#[cfg(feature = "ahcut")]
pub use svg_refusals::SvgExternalImage;
#[cfg(feature = "ahcut")]
pub(crate) use svg_refusals::SvgRefusals;
pub use text::{GeneratedTextInputEvent, INLINE_BOX_CHAR, TextBrush, TextInputData, TextLayout};
