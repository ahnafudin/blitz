//! Paint a [`blitz_dom::BaseDocument`] by pushing [`anyrender`] drawing commands into
//! an impl [`anyrender::PaintScene`].

#![allow(clippy::collapsible_if)]

mod blend;
mod color;
mod debug_overlay;
mod filters;
mod gradient;
mod kurbo_css;
mod layers;
mod render;
mod sizing;
mod text;
mod text_decorations;

use std::collections::HashMap;

use anyrender::{PaintScene, Scene};
use blitz_dom::{BaseDocument, NodeId, util::Color};
use render::BlitzDomPainter;

const FONT_EMBOLDEN_ENABLED: bool = cfg!(any(
    feature = "font-embolden",
    all(feature = "apple-font-embolden", target_os = "macos"),
    all(feature = "apple-font-embolden", target_os = "ios"),
));

/// The default color for text selection highlights
const SELECTION_COLOR: Color = Color::from_rgb8(180, 213, 255);

/// Pre-computed `Scene`s for each CustomWidget, keyed by `(document id, node id)`
type CustomWidgetSceneMap = HashMap<(usize, NodeId), Scene>;

/// Refusal payload for a visible host image whose pixels are not yet available to this renderer.
#[cfg(feature = "ahcut")]
pub const IMAGE_PAINT_PENDING: &str =
    "IMAGE_PAINT_PENDING:draw the shape in SVG or omit the image until image paint lands";

/// Paint a [`blitz_dom::BaseDocument`] by pushing drawing commands into
/// an impl [`anyrender::PaintScene`].
///
/// This function assumes that the styles and layout in the [`BaseDocument`] are already
/// resolved. Please ensure that this is the case before trying to paint.
///
/// The implementation of [`PaintScene`] is responsible for handling the commands that are pushed into it.
/// Generally this will involve executing them to draw a rasterized image/texture. But in some cases it may choose to
/// transform them to a vector format (e.g. SVG/PDF) or serialize them in raw form for later use.
pub fn paint_scene(
    scene: &mut impl PaintScene,
    doc: &mut BaseDocument,
    scale: f64,
    width: u32,
    height: u32,
    x_offset: u32,
    y_offset: u32,
) {
    // Run `.paint()` on every custom widget in the document (and all subdocuments) ahead of time.
    // This helps us avoid borrow-checker issues as we recurse down the tree (`.paint()` require `&mut self`).
    //
    // TODO: Take widget and sub-document visibility into account
    #[allow(unused_mut)]
    let mut custom_widget_scenes: CustomWidgetSceneMap = HashMap::new();
    #[cfg(feature = "custom-widget")]
    build_custom_widget_scenes(&mut custom_widget_scenes, doc, scene, scale);

    let generator = BlitzDomPainter::new(
        doc,
        scale,
        width,
        height,
        x_offset as f64,
        y_offset as f64,
        &custom_widget_scenes,
    );
    generator.paint_scene(scene);

    // println!(
    //     "Rendered using {} clips (depth: {}) (wanted: {})",
    //     CLIPS_USED.load(atomic::Ordering::SeqCst),
    //     CLIP_DEPTH_USED.load(atomic::Ordering::SeqCst),
    //     CLIPS_WANTED.load(atomic::Ordering::SeqCst)
    // );
}

/// Paint like [`paint_scene`], but refuse instead of drawing approximately: every URL filter must resolve to a
/// supported SVG Gaussian chain, whose author style sheets have known provenance; SVG text and SVG images that are
/// not `data:` URLs are refused; resources must be loaded and the document must not hold subdocuments. An error
/// means nothing that was pushed into `scene` may be shown.
#[cfg(feature = "ahcut")]
pub fn try_paint_scene(
    scene: &mut impl PaintScene,
    doc: &mut BaseDocument,
    scale: f64,
    width: u32,
    height: u32,
    x_offset: u32,
    y_offset: u32,
) -> Result<(), anyrender::filters::FilterError> {
    use anyrender::filters::FilterError;
    if let Some(message) = doc.svg_limit_refusal() {
        return Err(FilterError::Unsupported(message));
    }
    if let Some(message) = doc.script_style_refusal() {
        return Err(FilterError::Unsupported(message));
    }
    if !scale.is_finite() || scale <= 0.0 || width == 0 || height == 0 {
        return Err(FilterError::Invalid("paint viewport or scale"));
    }
    if doc.has_pending_critical_resources() {
        return Err(FilterError::Invalid("paint resources not ready"));
    }
    if doc.tree().iter().any(|(_, node)| {
        node.element_data()
            .is_some_and(|e| &*e.name.local == "iframe")
    }) {
        return Err(FilterError::Unsupported("subdocument paint"));
    }
    // Replacing an author sheet can remove every URL filter without any filter being resolved, so the document
    // is admitted before any widget or paint work.
    doc.validate_filter_stylesheet_provenance()?;
    // SVG text is shaped from usvg's font database, which a build without system fonts leaves empty.
    if doc.first_svg_text().is_some() {
        return Err(FilterError::Unsupported(
            "SVG text is not drawn: <text>, <tspan>, <textPath> and <tref> are refused",
        ));
    }
    // usvg did not load it (the resolver opens nothing).
    if doc.first_svg_external_image().is_some() {
        return Err(FilterError::Unsupported(
            "SVG image source: an image inside SVG may come only from a data: URL",
        ));
    }

    #[allow(unused_mut)]
    let mut custom_widget_scenes: CustomWidgetSceneMap = HashMap::new();
    #[cfg(feature = "custom-widget")]
    build_custom_widget_scenes(&mut custom_widget_scenes, doc, scene, scale);

    let has_image_facts = doc.has_image_facts();
    let mut generator = BlitzDomPainter::new(
        doc,
        scale,
        width,
        height,
        x_offset as f64,
        y_offset as f64,
        &custom_widget_scenes,
    );
    generator.checked_filters = true;
    // A cached image may be detached, hidden or superseded. Use the same clipped paint walk to
    // determine whether its brush is needed before publishing a single command to the caller.
    if has_image_facts {
        generator.paint_scene(&mut anyrender::NullScenePainter);
        if let Some(error) = generator.filter_error.borrow_mut().take() {
            return Err(error);
        }
    }
    generator.paint_scene(scene);
    match generator.filter_error.borrow_mut().take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
#[cfg(feature = "custom-widget")]
fn build_custom_widget_scenes(
    custom_widget_scenes: &mut CustomWidgetSceneMap,
    doc: &mut BaseDocument,
    render_ctx: &mut impl anyrender::RenderContext,
    scale: f64,
) {
    let doc_id = doc.id();

    // Process scenes for every custom widget in the document
    let custom_widget_node_ids = doc.custom_widget_node_ids();
    for node_id in custom_widget_node_ids.into_iter() {
        if let Some(scene) = process_custom_widget_node(doc, render_ctx, node_id, scale) {
            custom_widget_scenes.insert((doc_id, node_id), scene);
        }
    }

    // Recurse into sub documents
    let sub_document_node_ids = doc.sub_document_node_ids();
    for node_id in sub_document_node_ids.into_iter() {
        if let Some(sub_doc) = doc.get_node_mut(node_id).and_then(|node| node.subdoc_mut()) {
            let mut inner = sub_doc.inner_mut();
            build_custom_widget_scenes(custom_widget_scenes, &mut inner, render_ctx, scale);
        }
    }
}

#[cfg(feature = "custom-widget")]
fn process_custom_widget_node(
    doc: &mut BaseDocument,
    render_ctx: &mut impl anyrender::RenderContext,
    node_id: NodeId,
    scale: f64,
) -> Option<Scene> {
    use blitz_dom::node::{CustomWidgetStatus, ProxyRenderContext};

    let node = doc.get_node_mut(node_id)?;
    let width = (node.final_layout().size.width as f64 * scale) as u32;
    let height = (node.final_layout().size.height as f64 * scale) as u32;

    if width == 0 || height == 0 {
        return None;
    }

    let style = (*node.stylo_element_data().primary_styles()?).clone();
    let element = node.data.downcast_element_mut()?;
    let widget_data = element.custom_widget_data_mut()?;

    let mut render_ctx = ProxyRenderContext {
        inner: render_ctx,
        resource_ids: &mut widget_data.active_resource_ids,
    };

    if widget_data.status == CustomWidgetStatus::Suspended {
        widget_data.widget.can_create_surfaces(&mut render_ctx);
        widget_data.status = CustomWidgetStatus::Active;
    }

    let widget_scene = widget_data
        .widget
        .paint(&mut render_ctx, &style, width, height, scale);

    Some(widget_scene)
}
