//! The filters of one box in a checked paint (the `ahcut` feature): resolved, or refused with an error.
use super::{BlitzDomPainter, ElementCx};
use crate::filters::{StyloFilter, convert_filters_for_node};
use anyrender::filters::{Filter, FilterError, GaussianBlurXY};
use anyrender::{BoxShadowGeometry, BoxShadowKind};
use blitz_dom::node::SpecialElementData;
use blitz_dom::{Node, NodeId};
use kurbo::{Affine, Rect, Shape};
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
    /// Source ink in the node's local device pixels, before its own filter.
    /// Unknown extents keep the viewport crop and cannot justify culling a blur source.
    pub(super) fn source_ink(&self, node: &Node) -> Option<Rect> {
        if let Some(bounds) = self.ink_bounds.borrow().get(&node.id) {
            return *bounds;
        }
        let bounds = node.element_data().and_then(|_| {
            let cx = self.element_cx(node, *node.final_layout(), Affine::IDENTITY, None);
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
        // Layout overflow does not bound glyph overhangs, outside markers, embedded
        // scenes, or a descendant's filter output. Keep their conservative crop.
        if !matches!(self.element.special_data, SpecialElementData::None)
            || self.list_item.is_some()
            || self
                .element
                .inline_layout_data
                .as_ref()
                .is_some_and(|text| !text.text.is_empty())
            || self.node.scroll_offset().x != 0.0
            || self.node.scroll_offset().y != 0.0
        {
            return None;
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
                Some(bounds.union(transform.transform_rect_bbox(self.context.source_ink(node)?)))
            });
        });
        bounds
    }
}
