//! The filters of one box in a checked paint (the `ahcut` feature): resolved, or refused with an error.
use super::BlitzDomPainter;
use crate::filters::{StyloFilter, convert_filters_for_node};
use anyrender::filters::{Filter, FilterError};
use blitz_dom::NodeId;
use kurbo::Rect;
use std::sync::Arc;

type Filters = (Option<Arc<Filter>>, Option<Arc<Filter>>);

impl BlitzDomPainter<'_, '_> {
    /// The first refusal is kept in `filter_error` and the box is drawn unfiltered; `try_paint_scene` then
    /// returns the error, so nothing of the scene is shown.
    pub(super) fn resolve_checked_filters(
        &self,
        node: NodeId,
        bounds: Rect,
        filters: &[StyloFilter],
        backdrop: &[StyloFilter],
    ) -> Filters {
        if !backdrop.is_empty() {
            self.filter_error
                .borrow_mut()
                .get_or_insert(FilterError::Unsupported("backdrop filter"));
        }
        let filter = match convert_filters_for_node(filters, self.dom, node, bounds, self.scale) {
            Ok(filter) => filter.map(Arc::new),
            Err(error) => {
                self.filter_error.borrow_mut().get_or_insert(error);
                None
            }
        };
        (filter, None)
    }
}
