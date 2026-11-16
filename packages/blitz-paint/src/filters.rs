use blitz_dom::util::ToColorColor as _;
use style::color::AbsoluteColor;
pub(crate) use style::computed_values::filter::single_value::T as StyloFilter;

use anyrender::filters::{Filter, FilterEffect};

pub(crate) fn convert_filters(filters: &[StyloFilter]) -> Option<Filter> {
    if filters.is_empty() {
        return None;
    }
    // A `url()` member, which Stylo yields only with its `ahcut` feature, is not resolved here. Per Filter Effects,
    // a reference that cannot be resolved disables the whole list rather than leaving the other members applied.
    if filters
        .iter()
        .any(|filter| matches!(filter, StyloFilter::Url(_)))
    {
        return None;
    }

    Some(Filter::linear_list(
        filters.iter().filter_map(convert_single_filter),
    ))
}

pub(crate) fn convert_single_filter(filter: &StyloFilter) -> Option<FilterEffect> {
    Some(match filter {
        StyloFilter::Blur(radius) => FilterEffect::blur(radius.px()),
        StyloFilter::Brightness(amount) => FilterEffect::brightness(amount.0),
        StyloFilter::Contrast(amount) => FilterEffect::contrast(amount.0),
        StyloFilter::Grayscale(amount) => FilterEffect::grayscale(amount.0),
        StyloFilter::HueRotate(angle) => FilterEffect::hue_rotate(angle.radians()),
        StyloFilter::Invert(amount) => FilterEffect::invert(amount.0),
        StyloFilter::Opacity(amount) => FilterEffect::opacity(amount.0),
        StyloFilter::Saturate(amount) => FilterEffect::saturate(amount.0),
        StyloFilter::Sepia(amount) => FilterEffect::sepia(amount.0),
        StyloFilter::DropShadow(shadow) => FilterEffect::drop_shadow(
            shadow.horizontal.px(),
            shadow.vertical.px(),
            shadow.blur.px(),
            // TODO: pass in correct currentColor
            shadow
                .color
                .resolve_to_absolute(&AbsoluteColor::BLACK)
                .as_color_color(),
        ),
        StyloFilter::Url(_) => return None,
    })
}

/// The filter of a box in a checked paint: one `url()` member that resolves to a supported SVG Gaussian chain.
/// Any other list is refused with an error, never partly applied.
#[cfg(feature = "ahcut")]
pub(crate) fn convert_filters_for_node(
    filters: &[StyloFilter],
    doc: &blitz_dom::BaseDocument,
    node: blitz_dom::NodeId,
    bounds: kurbo::Rect,
    scale: f64,
) -> Result<Option<Filter>, anyrender::filters::FilterError> {
    use anyrender::filters::{FilterError, GaussianBlurXY};
    use kurbo::Affine;
    if filters.is_empty() {
        return Ok(None);
    }
    if filters.len() != 1 {
        return Err(FilterError::Unsupported("CSS filter list"));
    }
    let StyloFilter::Url(url) = &filters[0] else {
        return Err(FilterError::Unsupported("CSS filter function"));
    };
    let css_bounds = Affine::scale(1.0 / scale).transform_rect_bbox(bounds);
    let graph = doc.resolve_svg_filter(url, node, css_bounds)?;
    let stages = graph
        .checked_gaussian_stages()?
        .into_iter()
        .map(|blur| {
            GaussianBlurXY::new(
                blur.std_deviation.map(|sigma| sigma * scale as f32),
                Affine::scale(scale).transform_rect_bbox(blur.region),
                blur.linear_rgb,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let final_region = graph
        .output_region()
        .ok_or(FilterError::Invalid("missing final crop"))?;
    Ok(Some(Filter::gaussian_chain(
        &stages,
        Affine::scale(scale).transform_rect_bbox(final_region),
    )?))
}
