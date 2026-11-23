//! Geometry queries use the same SVG serialization and usvg geometry as native paint.
use crate::{BaseDocument, NodeId};

impl BaseDocument {
    /// Object-space fill bounds and user-to-SVG-viewport transform, never fabricated geometry.
    pub fn svg_bbox(&mut self, node_id: NodeId) -> Result<[f64; 4], String> {
        self.svg_geometry(node_id).map(|(bounds, _)| bounds)
    }

    pub fn svg_ctm(&mut self, node_id: NodeId) -> Result<[f64; 6], String> {
        let (_, mut matrix) = self.svg_geometry(node_id)?;
        let node = self
            .get_node(node_id)
            .ok_or("SVG element no longer exists")?;
        if node
            .element_data()
            .is_some_and(|el| el.name.local == markup5ever::local_name!("svg"))
        {
            // The outermost SVG maps to the document viewport, unlike child graphics.
            let mut current = Some(node_id);
            while let Some(id) = current {
                let ancestor = self.get_node(id).ok_or("SVG ancestor no longer exists")?;
                if id != node_id
                    && ancestor
                        .element_data()
                        .is_some_and(|el| el.name.ns == markup5ever::ns!(svg))
                {
                    return Err("Nested SVG viewport CTM is not supported".into());
                }
                if ancestor.transform().is_some() {
                    return Err("SVG root CTM with outer CSS transforms is not supported".into());
                }
                current = ancestor.containing_block();
            }
            let (position, layout) = self.physical_unrounded_geometry(node_id);
            let scroll = self.viewport_scroll();
            matrix[4] +=
                f64::from(position.x + layout.border.left + layout.padding.left) - scroll.x;
            matrix[5] += f64::from(position.y + layout.border.top + layout.padding.top) - scroll.y;
        }
        Ok(matrix)
    }

    fn svg_geometry(&mut self, node_id: NodeId) -> Result<([f64; 4], [f64; 6]), String> {
        #[cfg(feature = "svg")]
        {
            self.resolve_svg_geometry(node_id)
        }
        #[cfg(not(feature = "svg"))]
        {
            let _ = node_id;
            Err("SVG support is disabled".into())
        }
    }

    #[cfg(feature = "svg")]
    fn resolve_svg_geometry(&mut self, node_id: NodeId) -> Result<([f64; 4], [f64; 6]), String> {
        use markup5ever::{local_name, ns};
        self.resolve(0.0);
        let node = self
            .get_node(node_id)
            .ok_or("SVG element no longer exists")?;
        if node.element_data().is_none_or(|el| el.name.ns != ns!(svg)) {
            return Err("Geometry requires an SVG element".into());
        }
        let existing_id = node
            .attr(local_name!("id"))
            .filter(|id| !id.is_empty())
            .map(str::to_owned);
        let mut viewport_id = node_id;
        loop {
            let candidate = self
                .get_node(viewport_id)
                .ok_or("SVG ancestor no longer exists")?;
            if candidate
                .element_data()
                .is_some_and(|el| el.name.ns == ns!(svg) && el.name.local == local_name!("svg"))
            {
                break;
            }
            viewport_id = candidate.parent.ok_or("SVG element has no viewport")?;
        }
        let viewport = self.get_node(viewport_id).unwrap();
        let original = viewport.svg_source();
        let mut marker = "__blitz_geometry_target__".to_owned();
        while original.contains(&marker) {
            marker.push('_');
        }
        let source = if let Some(id) = existing_id {
            marker = id;
            original
        } else {
            viewport.svg_source_with_target(Some((node_id, &marker)))
        };
        let parsed = crate::util::parse_svg(
            source.as_bytes(),
            None,
            #[cfg(feature = "ahcut")]
            self.svg_image_budget.as_ref(),
        );
        #[cfg(feature = "ahcut")]
        if let Some(message) = self.svg_limit_refusal() {
            return Err(message.into());
        }
        let svg = parsed.map_err(|error| format!("SVG geometry parse failed: {error}"))?;
        let layout = viewport.final_layout();
        let width = layout.size.width
            - layout.border.left
            - layout.border.right
            - layout.padding.left
            - layout.padding.right;
        let height = layout.size.height
            - layout.border.top
            - layout.border.bottom
            - layout.padding.top
            - layout.padding.bottom;
        if width <= 0.0 || height <= 0.0 {
            return Err("SVG viewport has no rendered area".into());
        }
        let viewport_transform = svg.viewport_transform(width, height);
        // usvg has no box for an element without geometry; SVG's is empty, at the origin.
        let empty = || usvg::Rect::from_xywh(0.0, 0.0, 0.0, 0.0);
        let (bounds, transform) = if viewport_id == node_id {
            let user_to_tree = svg.user_to_tree_transform();
            let inverse = user_to_tree
                .invert()
                .ok_or("SVG viewBox transform is singular")?;
            let bounds = svg
                .tree
                .root()
                .bounding_box()
                .or_else(empty)
                .and_then(|bounds| bounds.transform(inverse))
                .ok_or("SVG bounds are not finite")?;
            (bounds, viewport_transform.pre_concat(user_to_tree))
        } else {
            let target = svg
                .tree
                .node_by_id(&marker)
                .ok_or("SVG element has no renderable geometry")?;
            (
                target
                    .bounding_box()
                    .or_else(empty)
                    .ok_or("SVG bounds are not finite")?,
                viewport_transform.pre_concat(target.abs_transform()),
            )
        };
        Ok((
            [
                bounds.x() as f64,
                bounds.y() as f64,
                bounds.width() as f64,
                bounds.height() as f64,
            ],
            [
                transform.sx as f64,
                transform.ky as f64,
                transform.kx as f64,
                transform.sy as f64,
                transform.tx as f64,
                transform.ty as f64,
            ],
        ))
    }
}
