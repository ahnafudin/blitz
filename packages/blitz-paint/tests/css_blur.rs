//! Checked CSS blur paints into the same Gaussian graph as an authored SVG filter.
#![cfg(feature = "ahcut")]

use anyrender::Scene;
use anyrender::filters::FilterError;
use anyrender::recording::RenderCommand;
use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use blitz_traits::shell::Viewport;

fn paint(filter: &str, scale: f64) -> Result<Scene, FilterError> {
    let html = format!(
        "<!doctype html><style>body{{margin:0}}
         #target{{position:absolute;left:20px;top:32px;width:24px;height:12px;
         background:red;box-shadow:0 1px 3px blue;filter:{filter}}}
         #child{{position:absolute;left:46px;top:2px;width:6px;height:6px;background:lime}}
         </style><div id=target><div id=child></div></div>"
    );
    let mut doc = HtmlDocument::from_html(
        &html,
        DocumentConfig {
            viewport: Some(Viewport {
                window_size: (240, 200),
                hidpi_scale: scale as f32,
                ..Viewport::default()
            }),
            ..DocumentConfig::default()
        },
    );
    doc.resolve(0.0);
    let mut scene = Scene::new();
    blitz_paint::try_paint_scene(&mut scene, &mut doc, scale, 240, 200, 0, 0)?;
    Ok(scene)
}

#[test]
fn a_css_length_is_the_srgb_gaussian_deviation_on_both_axes() {
    for (length, scale, sigma) in [("14px", 1.0, 14.0), ("7px", 2.0, 14.0)] {
        let scene = paint(&format!("blur({length})"), scale).unwrap();
        let filters: Vec<_> = scene
            .commands
            .iter()
            .filter_map(|command| match command {
                RenderCommand::PushLayer(layer) => layer.filter.as_deref(),
                _ => None,
            })
            .collect();
        assert_eq!(filters.len(), 1);
        let stages = filters[0].checked_gaussian_stages().unwrap();
        assert_eq!(stages.len(), 1);
        assert_eq!(stages[0].std_deviation, [sigma, sigma]);
        assert!(!stages[0].linear_rgb);
        let crop = filters[0].output_region().unwrap();
        // The positioned child extends to 52 CSS pixels in the parent's coordinates.
        // Keep a visible halo outside its far edge, including at device scale two.
        assert!(crop.x1 >= 52.0 * scale + f64::from(sigma) && crop.x0 <= -f64::from(sigma));
        assert_eq!(stages[0].region, crop);
    }
}

#[test]
fn blur_zero_and_the_css_default_do_not_refuse_paint() {
    for filter in ["blur(0px)", "blur()"] {
        assert!(paint(filter, 1.0).is_ok(), "{filter}");
    }
}

#[test]
fn the_outer_shadow_is_part_of_the_css_filters_source() {
    let scene = paint("blur(4px)", 1.0).unwrap();
    let mut filters = Vec::new();
    let mut shadow_count = 0;
    for command in scene.commands {
        match command {
            RenderCommand::PushLayer(layer) => filters.push(layer.filter.is_some()),
            RenderCommand::PushClipLayer(_) | RenderCommand::PushMaskLayer(_) => {
                filters.push(false)
            }
            RenderCommand::PopLayer => {
                filters.pop().expect("balanced layers");
            }
            RenderCommand::BoxShadow(_) => {
                assert!(
                    filters.contains(&true),
                    "shadow painted outside the CSS blur"
                );
                shadow_count += 1;
            }
            _ => {}
        }
    }
    assert_eq!(shadow_count, 1);
    assert!(filters.is_empty());
}

#[test]
fn unsupported_filter_members_keep_the_whole_list_refusal() {
    for value in [
        "contrast(2)",
        "blur(2px) contrast(2)",
        "drop-shadow(1px 2px 3px black)",
    ] {
        assert!(
            matches!(paint(value, 1.0), Err(FilterError::Unsupported(_))),
            "{value}"
        );
    }
}
