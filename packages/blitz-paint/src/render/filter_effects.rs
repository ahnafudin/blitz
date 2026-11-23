//! The filters of one box in a checked paint (the `ahcut` feature): resolved, or refused with an error.
use super::{BlitzDomPainter, ElementCx};
use crate::filters::{StyloFilter, convert_filters_for_node};
use anyrender::filters::{Filter, FilterError, GaussianBlurXY};
use anyrender::{BoxShadowGeometry, BoxShadowKind};
use blitz_dom::node::{SpecialElementData, TextBrush};
use blitz_dom::{Node, NodeId};
use kurbo::{Affine, Point, Rect, Shape, Vec2};
use parley::GlyphRun;
use std::sync::Arc;

type Filters = (Option<Arc<Filter>>, Option<Arc<Filter>>);

#[derive(Clone, Copy)]
pub(super) struct CssBlur {
    sigma: f32,
    region: Rect,
    pub(super) source_clip: Rect,
}

impl CssBlur {
    pub(super) fn from_css(
        filters: &[StyloFilter],
        scale: f64,
        local_to_screen: Affine,
        output_clip: Rect,
    ) -> Option<Self> {
        let [StyloFilter::Blur(length)] = filters else {
            return None;
        };
        let sigma = length.px() * scale as f32;
        // Each output pixel reads the same three-sigma halo as the checked converter.
        // A CSS function includes overflowing descendants and has no author-supplied crop.
        let halo = 3.0 * f64::from(sigma);
        let [a, b, c, d, _, _] = local_to_screen.as_coeffs();
        let source_clip =
            output_clip.inflate(halo * (a.abs() + c.abs()), halo * (b.abs() + d.abs()));
        let region = local_to_screen
            .inverse()
            .transform_rect_bbox(output_clip)
            .inflate(halo, halo);
        Some(Self {
            sigma,
            region,
            source_clip,
        })
    }

    pub(super) fn with_ink(mut self, ink: Option<Rect>) -> Self {
        if let Some(ink) = ink {
            let halo = 3.0 * f64::from(self.sigma);
            self.region = self.region.intersect(ink.inflate(halo, halo));
        }
        self
    }

    fn filter(self) -> Result<Option<Filter>, FilterError> {
        // CSS shorthand functions operate in sRGB. For blur() the CSS length is
        // the Gaussian deviation itself, including when the computed length is zero.
        let stage = GaussianBlurXY::new([self.sigma; 2], self.region, false)?;
        Filter::gaussian_chain(&[stage], self.region).map(Some)
    }
}

impl BlitzDomPainter<'_, '_> {
    pub(super) fn subtree_has_blur(&self, node: &Node) -> bool {
        if let Some(cached) = self.contains_blur.borrow().get(&node.id) {
            return *cached;
        }
        let own = node.element_data().is_some()
            && node.primary_styles().is_some_and(|styles| {
                matches!(&styles.get_effects().filter.0[..], [StyloFilter::Blur(_)])
            });
        let children = node.paint_children.borrow();
        let hoisted = node
            .stacking_context
            .iter()
            .flat_map(|context| context.children.iter().map(|child| child.node_id));
        let has_blur = own
            || children.iter().flatten().copied().chain(hoisted).any(|id| {
                self.dom
                    .get_node(id)
                    .is_some_and(|child| self.subtree_has_blur(child))
            });
        self.contains_blur.borrow_mut().insert(node.id, has_blur);
        has_blur
    }

    /// Source ink in the node's local device pixels, before its own filter.
    /// Unknown extents keep the viewport crop and cannot justify culling a blur source.
    pub(super) fn source_ink(&self, node: &Node, local_to_screen: Affine) -> Option<Rect> {
        if let Some(bounds) = self.ink_bounds.borrow().get(&node.id) {
            return *bounds;
        }
        let bounds = node.element_data().and_then(|_| {
            let cx = self.element_cx(node, *node.final_layout(), local_to_screen, None);
            cx.source_ink()
        });
        self.ink_bounds.borrow_mut().insert(node.id, bounds);
        bounds
    }

    /// The first refusal is kept in `filter_error` and the box is drawn unfiltered; `try_paint_scene` then
    /// returns the error, so nothing of the scene is shown.
    pub(super) fn resolve_checked_filters(
        &self,
        node: NodeId,
        bounds: Rect,
        filters: &[StyloFilter],
        backdrop: &[StyloFilter],
        css_blur: Option<CssBlur>,
    ) -> Filters {
        if !backdrop.is_empty() {
            self.filter_error
                .borrow_mut()
                .get_or_insert(FilterError::Unsupported("backdrop filter"));
        }
        let checked = match css_blur {
            Some(blur) => blur.filter(),
            None => convert_filters_for_node(filters, self.dom, node, bounds, self.scale),
        };
        let filter = match checked {
            Ok(filter) => filter.map(Arc::new),
            Err(error) => {
                self.filter_error.borrow_mut().get_or_insert(error);
                None
            }
        };
        (filter, None)
    }
}

impl ElementCx<'_, '_> {
    fn source_ink(&self) -> Option<Rect> {
        // Collapsed table borders and row backgrounds are drawn before the content clip.
        if matches!(self.element.special_data, SpecialElementData::TableRoot(_)) {
            return None;
        }
        let border = self.frame.border_box_shape();
        let mut ink = self.frame.outline().bounding_box().union(border.rect);
        for shadow in self
            .style
            .get_effects()
            .box_shadow
            .0
            .iter()
            .filter(|s| !s.inset)
        {
            let (offset, spread, std_dev) = self.shadow_dimensions(shadow);
            let shadow = BoxShadowGeometry::new(
                &border,
                offset,
                spread,
                std_dev,
                BoxShadowKind::OutsetUnclipped,
            );
            ink = ink.union(shadow.area.bounding_box());
        }
        if self.devtools.show_layout {
            ink = ink.union(border.rect.inflate(self.scale, self.scale));
        }
        if self.should_clip_node(self.node, &self.style) {
            return Some(ink);
        }
        // Outside markers, embedded scenes and nested filters can extend beyond layout.
        if !matches!(self.element.special_data, SpecialElementData::None)
            || self.list_item.is_some()
            || self.node.scroll_offset().x != 0.0
            || self.node.scroll_offset().y != 0.0
        {
            return None;
        }
        if self.node.flags.is_inline_root() {
            ink = self.include_text_ink(ink)?;
        }
        let mut bounds = Some(ink.union(*self.node.scrollable_overflow()));
        self.for_each_paint_child(Affine::IDENTITY, |id, parent| {
            bounds = bounds.and_then(|bounds| {
                let node = self.dom.get_node(id)?;
                let styles = node.primary_styles()?;
                if !styles.get_effects().filter.0.is_empty() {
                    return None;
                }
                let transform = self.node_transform(node, parent);
                let child_ink = self.context.source_ink(node, self.transform * transform)?;
                Some(bounds.union(transform.transform_rect_bbox(child_ink)))
            });
        });
        bounds
    }

    fn include_text_ink(&self, mut ink: Rect) -> Option<Rect> {
        let text = self.element.inline_layout_data.as_ref()?;
        if self.selection_ranges.contains_key(&self.node.id) {
            return None;
        }
        let layout = self.node.final_layout();
        let inset = layout.padding + layout.border;
        let origin = self.inline_text_origin(text, Point::new(inset.left.into(), inset.top.into()));
        // Map one device pixel for hinting and antialiasing back to the local axes.
        let inverse = self.transform.inverse();
        if !inverse.is_finite() {
            return None;
        }
        let [a, b, c, d, _, _] = inverse.as_coeffs();
        // Optional stem darkening expands the outline in addition to antialiasing.
        let margin = if crate::FONT_EMBOLDEN_ENABLED {
            2.0
        } else {
            1.0
        };
        for line in text.layout.lines() {
            for glyph_run in crate::text::glyph_runs(&line) {
                let mut id = glyph_run.style().brush.id;
                loop {
                    let node = self.dom.get_node(id)?;
                    let style = node.primary_styles()?;
                    if !style.get_text().text_decoration_line.is_empty()
                        || !style.get_inherited_text().text_shadow.0.is_empty()
                    {
                        return None;
                    }
                    if id == self.node.id {
                        break;
                    }
                    id = node.parent?;
                }
                let run_ink = glyph_run_ink(&glyph_run)?
                    .inflate(margin * (a.abs() + c.abs()), margin * (b.abs() + d.abs()));
                ink = ink.union(run_ink + origin);
            }
        }
        Some(ink)
    }
}

fn glyph_run_ink(glyph_run: &GlyphRun<'_, TextBrush>) -> Option<Rect> {
    use skrifa::raw::{FontRef, TableProvider as _, types::Tag};
    let run = glyph_run.run();
    let font = FontRef::from_index(run.font().data.as_ref(), run.font().index).ok()?;
    // The head bounds cover outline glyphs at the default variation coordinates.
    // Bitmap and colour paints can have ink outside these bounds.
    if run
        .normalized_coords()
        .iter()
        .any(|coord| coord.to_bits() != 0)
        || [b"COLR", b"SVG ", b"sbix", b"CBDT", b"EBDT", b"bdat"]
            .into_iter()
            .any(|tag| font.table_data(Tag::new(tag)).is_some())
    {
        return None;
    }
    let head = font.head().ok()?;
    if head.units_per_em() == 0 || head.x_min() > head.x_max() || head.y_min() > head.y_max() {
        return None;
    }
    let scale = f64::from(run.font_size()) / f64::from(head.units_per_em());
    let face = Rect::new(
        f64::from(head.x_min()) * scale,
        -f64::from(head.y_max()) * scale,
        f64::from(head.x_max()) * scale,
        -f64::from(head.y_min()) * scale,
    );
    let face = crate::text::glyph_transform(glyph_run)
        .unwrap_or(Affine::IDENTITY)
        .transform_rect_bbox(face);
    let mut ink = crate::text::glyph_run_rect(glyph_run);
    for glyph in glyph_run.positioned_glyphs() {
        ink = ink.union(face + Vec2::new(glyph.x.into(), glyph.y.into()));
    }
    ink.is_finite().then_some(ink)
}
