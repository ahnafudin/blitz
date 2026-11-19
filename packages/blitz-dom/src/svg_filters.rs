//! Resolves a CSS `url()` filter to a same-document SVG `<filter>` of one or two serial `feGaussianBlur`s on
//! `SourceGraphic`, live on every paint (the `ahcut` feature).

use crate::{BaseDocument, Node, NodeId};
use anyrender::filters::{Filter, FilterError, GaussianBlurXY};
use kurbo::Rect;
use style::values::computed::url::ComputedUrl;
use svgtypes::{Length, LengthUnit, NumberListParser};

fn attr<'a>(node: &'a Node, name: &str) -> Option<&'a str> {
    node.attrs()?
        .iter()
        .find(|a| &*a.name.local == name)
        .map(|a| a.value.as_str())
}
fn sigma_pair(text: &str) -> Result<[f32; 2], FilterError> {
    let values = NumberListParser::from(text)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| FilterError::Invalid("SVG number list"))?;
    let pair = match values.as_slice() {
        [x] => [*x, *x],
        [x, y] => [*x, *y],
        _ => return Err(FilterError::Invalid("stdDeviation arity")),
    };
    if pair.iter().any(|v| !v.is_finite() || *v < 0.0 || *v > 64.0) {
        return Err(FilterError::Invalid("stdDeviation range"));
    }
    Ok([pair[0] as f32, pair[1] as f32])
}
fn decode_fragment(text: &str) -> Result<String, FilterError> {
    let mut bytes = Vec::new();
    let mut input = text.as_bytes().iter().copied();
    while let Some(b) = input.next() {
        if b == b'%' {
            let hi = input.next().and_then(|b| (b as char).to_digit(16));
            let lo = input.next().and_then(|b| (b as char).to_digit(16));
            let (Some(hi), Some(lo)) = (hi, lo) else {
                return Err(FilterError::Invalid("URL fragment escape"));
            };
            bytes.push((hi * 16 + lo) as u8);
        } else {
            bytes.push(b);
        }
    }
    let id = String::from_utf8(bytes).map_err(|_| FilterError::Invalid("URL fragment UTF8"))?;
    if id.is_empty() || id.contains('\0') {
        return Err(FilterError::Invalid("empty/NUL filter ID"));
    }
    Ok(id)
}
fn region_length(value: &str, bbox: f64, object_bbox: bool) -> Result<f64, FilterError> {
    let length = value
        .trim()
        .parse::<Length>()
        .map_err(|_| FilterError::Invalid("filter region length"))?;
    if !length.number.is_finite() {
        return Err(FilterError::Invalid("nonfinite region"));
    }
    match (object_bbox, length.unit) {
        (true, LengthUnit::Percent) => Ok(length.number * bbox / 100.0),
        (true, LengthUnit::None) => Ok(length.number * bbox),
        (false, LengthUnit::None | LengthUnit::Px) => Ok(length.number),
        _ => Err(FilterError::Unsupported("filter region units")),
    }
}
fn svg_element(node: &Node, name: &str) -> bool {
    node.element_data()
        .is_some_and(|e| &*e.name.ns == "http://www.w3.org/2000/svg" && &*e.name.local == name)
}
fn validate_attrs(node: &Node, allowed: &[&str]) -> Result<(), FilterError> {
    for a in node.attrs().unwrap_or_default() {
        if !a.name.ns.is_empty() || !allowed.contains(&a.name.local.as_ref()) {
            return Err(FilterError::Unsupported(
                "SVG definition attribute/style/reference",
            ));
        }
    }
    Ok(())
}

// A serial chain of at most two stages, not a general SVG graph. Names are resolved before paint.
fn validate_chain_links(links: &[(Option<&str>, Option<&str>)]) -> Result<(), FilterError> {
    if links.len() == 1 {
        return match links[0].0 {
            None | Some("SourceGraphic") => Ok(()),
            _ => Err(FilterError::Unsupported("filter input/result reference")),
        };
    }
    if links.len() != 2 {
        return Err(FilterError::Unsupported(
            "Gaussian chain longer than two stages",
        ));
    }
    fn named(name: Option<&str>) -> Result<&str, FilterError> {
        let name = name.ok_or(FilterError::Unsupported("named Gaussian result required"))?;
        if name.is_empty()
            || name.chars().any(char::is_whitespace)
            || [
                "SourceGraphic",
                "SourceAlpha",
                "BackgroundImage",
                "BackgroundAlpha",
                "FillPaint",
                "StrokePaint",
            ]
            .contains(&name)
        {
            return Err(FilterError::Unsupported("Gaussian result name"));
        }
        Ok(name)
    }
    let first = named(links[0].1)?;
    // The terminal result need not be named; if present it must be valid and unique.
    let second = links[1].1.map(|name| named(Some(name))).transpose()?;
    if second == Some(first) || links[0].0 != Some("SourceGraphic") || links[1].0 != Some(first) {
        return Err(FilterError::Unsupported("filter input/result reference"));
    }
    Ok(())
}

/// Primitive subregions use primitiveUnits; omitted components inherit the final filter region.
fn stage_region(
    values: [Option<&str>; 4],
    final_region: Rect,
    bounds: Rect,
    object_bbox: bool,
) -> Result<Rect, FilterError> {
    let origin_x = if object_bbox { bounds.x0 } else { 0.0 };
    let origin_y = if object_bbox { bounds.y0 } else { 0.0 };
    let x = values[0]
        .map(|v| region_length(v, bounds.width(), object_bbox).map(|v| v + origin_x))
        .transpose()?
        .unwrap_or(final_region.x0);
    let y = values[1]
        .map(|v| region_length(v, bounds.height(), object_bbox).map(|v| v + origin_y))
        .transpose()?
        .unwrap_or(final_region.y0);
    let width = values[2]
        .map(|v| region_length(v, bounds.width(), object_bbox))
        .transpose()?
        .unwrap_or(final_region.width());
    let height = values[3]
        .map(|v| region_length(v, bounds.height(), object_bbox))
        .transpose()?
        .unwrap_or(final_region.height());
    let region = Rect::new(x, y, x + width, y + height);
    GaussianBlurXY::new([0.0, 0.0], region, false)?;
    Ok(region)
}

impl BaseDocument {
    /// Resolve the same-document chain of at most two `SourceGraphic` Gaussian stages, live on every paint.
    pub fn resolve_svg_filter(
        &self,
        reference: &ComputedUrl,
        consumer: NodeId,
        bounds: Rect,
    ) -> Result<Filter, FilterError> {
        if !self.tree().contains_key(consumer) {
            return Err(FilterError::Invalid("stale consumer"));
        }
        self.validate_filter_stylesheet_provenance()?;
        let mut ancestor = Some(consumer);
        while let Some(id) = ancestor {
            let node = &self.tree()[id];
            if let Some(styles) = node.primary_styles() {
                use style_traits::ToCss;
                let box_styles = styles.get_box();
                if box_styles.perspective.to_css_string() != "none" {
                    return Err(FilterError::Unsupported("perspective filter transform"));
                }
                if !box_styles.transform.0.is_empty() {
                    let (matrix, _) = box_styles
                        .transform
                        .to_transform_3d_matrix(None)
                        .map_err(|_| FilterError::Unsupported("filter transform reference box"))?;
                    if !matrix.is_2d() {
                        return Err(FilterError::Unsupported("3D filter transform"));
                    }
                }
                if let style::values::computed::Rotate::Rotate3D(x, y, _, _) = &box_styles.rotate {
                    if *x != 0.0 || *y != 0.0 {
                        return Err(FilterError::Unsupported("3D filter rotate"));
                    }
                }
            }
            ancestor = node.parent;
        }
        let mut target = reference
            .url()
            .ok_or(FilterError::Invalid("computed filter URL"))?
            .as_ref()
            .clone();
        let id = decode_fragment(
            target
                .fragment()
                .ok_or(FilterError::Unsupported("filter URL without fragment"))?,
        )?;
        target.set_fragment(None);
        let mut base = self.base_url().clone();
        base.set_fragment(None);
        if target != base {
            return Err(FilterError::Unsupported("external filter URL"));
        }
        let id = self
            .get_element_by_id(&id)
            .ok_or(FilterError::Invalid("missing filter definition"))?;
        let definition = &self.tree()[id];
        if !svg_element(definition, "filter") {
            return Err(FilterError::Unsupported("URL target is not SVG filter"));
        }
        validate_attrs(
            definition,
            &[
                "id",
                "x",
                "y",
                "width",
                "height",
                "filterUnits",
                "primitiveUnits",
                "color-interpolation-filters",
            ],
        )?;

        let mut primitives = Vec::new();
        for child in &definition.children {
            let node = &self.tree()[*child];
            if node.element_data().is_none()
                || svg_element(node, "title")
                || svg_element(node, "desc")
                || svg_element(node, "metadata")
            {
                continue;
            }
            if !svg_element(node, "feGaussianBlur") {
                return Err(FilterError::Unsupported("filter graph/primitive"));
            }
            if primitives.len() == 2 {
                return Err(FilterError::Unsupported(
                    "Gaussian chain longer than two stages",
                ));
            }
            validate_attrs(
                node,
                &[
                    "id",
                    "stdDeviation",
                    "in",
                    "result",
                    "edgeMode",
                    "color-interpolation-filters",
                    "x",
                    "y",
                    "width",
                    "height",
                ],
            )?;
            if node
                .children
                .iter()
                .any(|id| self.tree()[*id].element_data().is_some())
            {
                return Err(FilterError::Unsupported("SVG animation/primitive child"));
            }
            if attr(node, "edgeMode").is_some_and(|v| v != "none") {
                return Err(FilterError::Unsupported("Gaussian edgeMode"));
            }
            primitives.push(node);
        }
        if primitives.is_empty() {
            return Err(FilterError::Invalid("empty filter"));
        }
        let links: Vec<_> = primitives
            .iter()
            .map(|p| (attr(p, "in"), attr(p, "result")))
            .collect();
        validate_chain_links(&links)?;
        let object_bbox = match attr(definition, "filterUnits").unwrap_or("objectBoundingBox") {
            "objectBoundingBox" => true,
            "userSpaceOnUse" => false,
            _ => return Err(FilterError::Unsupported("filterUnits")),
        };
        let primitive_bbox = match attr(definition, "primitiveUnits").unwrap_or("userSpaceOnUse") {
            "userSpaceOnUse" => false,
            "objectBoundingBox" => true,
            _ => return Err(FilterError::Unsupported("primitiveUnits")),
        };
        let x = region_length(
            attr(definition, "x").unwrap_or("-10%"),
            bounds.width(),
            object_bbox,
        )?;
        let y = region_length(
            attr(definition, "y").unwrap_or("-10%"),
            bounds.height(),
            object_bbox,
        )?;
        let width = region_length(
            attr(definition, "width").unwrap_or("120%"),
            bounds.width(),
            object_bbox,
        )?;
        let height = region_length(
            attr(definition, "height").unwrap_or("120%"),
            bounds.height(),
            object_bbox,
        )?;
        let region = Rect::new(
            x + if object_bbox { bounds.x0 } else { 0.0 },
            y + if object_bbox { bounds.y0 } else { 0.0 },
            x + width + if object_bbox { bounds.x0 } else { 0.0 },
            y + height + if object_bbox { bounds.y0 } else { 0.0 },
        );

        let mut stages = Vec::with_capacity(primitives.len());
        for primitive in primitives {
            let mut sigma = sigma_pair(attr(primitive, "stdDeviation").unwrap_or("0"))?;
            if primitive_bbox {
                sigma[0] *= bounds.width() as f32;
                sigma[1] *= bounds.height() as f32;
            }
            let primitive_region = stage_region(
                [
                    attr(primitive, "x"),
                    attr(primitive, "y"),
                    attr(primitive, "width"),
                    attr(primitive, "height"),
                ],
                region,
                bounds,
                primitive_bbox,
            )?;
            // This Stylo pin has color-interpolation-filters only for Gecko, not Servo, so the value is read
            // here: the nearest SVG element from the primitive upwards that carries the presentation
            // attribute, else linearRGB. That equals the computed value only while no CSS sets the property,
            // because CSS outranks a presentation attribute and the property inherits. Author sheets are held
            // to that by the provenance check above; a style attribute anywhere on the way up is held to it
            // here. A class, or a style attribute that declares other properties, sets nothing and is admitted.
            let mut linear_rgb = true;
            let mut found = false;
            let mut ancestor = Some(primitive.id);
            while let Some(id) = ancestor {
                let n = &self.tree()[id];
                if let Some(style) = attr(n, "style") {
                    crate::filter_stylesheets::check_inline_style(style)?;
                }
                // Only SVG elements map the attribute to the property; on any other element it means nothing.
                let presentational = n
                    .element_data()
                    .is_some_and(|e| &*e.name.ns == "http://www.w3.org/2000/svg");
                if !found
                    && presentational
                    && let Some(color) = attr(n, "color-interpolation-filters")
                {
                    linear_rgb = match color {
                        "linearRGB" => true,
                        "sRGB" => false,
                        _ => return Err(FilterError::Unsupported("filter color space")),
                    };
                    found = true;
                }
                ancestor = n.parent;
            }

            stages.push(GaussianBlurXY::new(sigma, primitive_region, linear_rgb)?);
        }
        Filter::gaussian_chain(&stages, region)
    }

    /// The filter when it is exactly one Gaussian stage. A two-stage chain is refused, never truncated.
    pub fn resolve_svg_gaussian(
        &self,
        reference: &ComputedUrl,
        consumer: NodeId,
        bounds: Rect,
    ) -> Result<GaussianBlurXY, FilterError> {
        let graph = self.resolve_svg_filter(reference, consumer, bounds)?;
        let stages = graph.checked_gaussian_stages()?;
        if stages.len() != 1 {
            return Err(FilterError::Unsupported("single Gaussian accessor"));
        }
        Ok(stages[0].clone())
    }
}

#[cfg(test)]
mod chain_tests;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn svg_number_pairs_preserve_zero_axes_and_reject_junk() {
        assert_eq!(sigma_pair("12, 0").unwrap(), [12.0, 0.0]);
        assert_eq!(sigma_pair("0 12").unwrap(), [0.0, 12.0]);
        assert_eq!(sigma_pair("6").unwrap(), [6.0, 6.0]);
        assert_eq!(sigma_pair("0 0").unwrap(), [0.0, 0.0]);
        for bad in ["1 2 3", "NaN 0", "-1 0", "1 trailing"] {
            assert!(sigma_pair(bad).is_err());
        }
    }
    #[test]
    fn fragment_decode_once() {
        assert_eq!(decode_fragment("a%20b").unwrap(), "a b");
        assert_eq!(decode_fragment("a%2520b").unwrap(), "a%20b");
        assert!(decode_fragment("%zz").is_err());
        assert_eq!(decode_fragment("%C3%A9").unwrap(), "é");
        for bad in ["%", "%0", "%00", "%FF", ""] {
            assert!(decode_fragment(bad).is_err(), "{bad}");
        }
    }
}
