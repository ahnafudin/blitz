//! The filters of one box in a checked paint (the `ahcut` feature): resolved, or refused with an error.
use super::BlitzDomPainter;
use crate::filters::{StyloFilter, convert_filters_for_node};
use anyrender::filters::{Filter, FilterError, GaussianBlurXY};
use blitz_dom::NodeId;
use kurbo::{Affine, Rect};
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

    fn filter(self) -> Result<Option<Filter>, FilterError> {
        // CSS shorthand functions operate in sRGB. For blur() the CSS length is
        // the Gaussian deviation itself, including when the computed length is zero.
        let stage = GaussianBlurXY::new([self.sigma; 2], self.region, false)?;
        Filter::gaussian_chain(&[stage], self.region).map(Some)
    }
}

impl BlitzDomPainter<'_, '_> {
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
