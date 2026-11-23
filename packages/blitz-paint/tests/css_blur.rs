//! Checked CSS blur paints into the same Gaussian graph as an authored SVG filter.
#![cfg(feature = "ahcut")]

use anyrender::Scene;
use anyrender::filters::FilterError;
use anyrender::recording::RenderCommand;
use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use blitz_traits::shell::Viewport;
use kurbo::Shape;

fn paint(filter: &str, scale: f64) -> Result<Scene, FilterError> {
    let html = format!(
        "<!doctype html><style>body{{margin:0}}
         #target{{position:absolute;left:20px;top:32px;width:24px;height:12px;
         background:red;box-shadow:0 1px 3px blue;filter:{filter}}}
         #child{{position:absolute;left:46px;top:2px;width:6px;height:6px;background:lime}}
         </style><div id=target><div id=child></div></div>"
    );
    paint_html(&html, scale)
}

fn paint_html(html: &str, scale: f64) -> Result<Scene, FilterError> {
    paint_html_with_font(html, scale, None)
}

fn paint_html_with_font(html: &str, scale: f64, font: Option<&[u8]>) -> Result<Scene, FilterError> {
    let mut doc = HtmlDocument::from_html(
        html,
        DocumentConfig {
            font_ctx: font.map(blitz_dom::build_single_font_ctx),
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
fn outlines_and_outset_shadows_are_in_the_source_of_css_and_svg_filters() {
    for filter in ["blur(4px)", "url(#soft)"] {
        let scene = paint_html(
            &format!(
                r##"<!doctype html>
            <svg width="0" height="0"><defs><filter id="soft">
            <feGaussianBlur stdDeviation="4"/></filter></defs></svg>
            <div style="width:24px;height:12px;outline:3px solid red;
            box-shadow:0 1px 3px blue;filter:{filter}"></div>"##
            ),
            1.0,
        )
        .unwrap();
        assert_grouped_decoration(&scene);
    }
}

fn assert_grouped_decoration(scene: &Scene) {
    let mut filters = Vec::new();
    let mut shadow_count = 0;
    let mut outline_count = 0;
    for command in &scene.commands {
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
            RenderCommand::Fill(fill) if fill.shape.bounding_box().width() == 30.0 => {
                assert!(
                    filters.contains(&true),
                    "outline painted outside the filter"
                );
                outline_count += 1;
            }
            _ => {}
        }
    }
    assert_eq!(shadow_count, 1);
    assert_eq!(outline_count, 1);
    assert!(filters.is_empty());
}

#[test]
fn independent_small_boxes_only_allocate_their_own_ink_and_halo() {
    let scene = paint_html(
        "<!doctype html><style>
        .tile{position:absolute;top:40px;width:12px;height:9px;background:green;filter:blur(2px)}
        .tile i{position:absolute;left:15px;width:5px;height:4px;background:red;
        outline:2px solid blue;box-shadow:8px 0 4px black}
        </style><div class=tile style='left:40px'><i></i></div>
        <div class=tile style='left:120px'><i></i></div>",
        1.0,
    )
    .unwrap();
    let mut crops = 0;
    for command in scene.commands {
        let RenderCommand::PushLayer(layer) = command else {
            continue;
        };
        let Some(graph) = layer.filter else { continue };
        let crop = graph.output_region().unwrap();
        // The child's far edge (20) plus the shadow offset (8), the shadow halo
        // (6), and the filter halo (6) are all inside the target's local crop.
        assert!(crop.x0 <= -6.0 && crop.x1 >= 40.0, "{crop:?}");
        assert!(crop.width() <= 52.0 && crop.height() <= 38.0, "{crop:?}");
        crops += 1;
    }
    assert_eq!(crops, 2);
}

#[test]
fn transformed_blur_keeps_a_near_source_and_culls_a_distant_one() {
    let scene = paint_html(
        "<!doctype html><style>body{margin:0}
        #f{position:absolute;left:220px;top:20px;width:8px;height:8px;
        transform-origin:0 0;transform:rotate(90deg) scale(2);filter:blur(5px)}
        #f div{position:absolute;left:0;height:4px;background:red}
        </style><div id=f><div style='top:-15px;width:3px'></div>
        <div style='top:-60px;width:7px'></div></div>",
        1.0,
    )
    .unwrap();
    let mut near = false;
    let mut far = false;
    let mut saw_transform = false;
    for command in scene.commands {
        match command {
            RenderCommand::PushLayer(layer) if layer.filter.is_some() => {
                let [a, b, c, d, _, _] = layer.transform.as_coeffs();
                assert!(
                    a.abs() < 0.001
                        && d.abs() < 0.001
                        && (b - 2.0).abs() < 0.001
                        && (c + 2.0).abs() < 0.001
                );
                saw_transform = true;
            }
            RenderCommand::Fill(fill) => {
                let bounds = fill.shape.bounding_box();
                if bounds.height() == 4.0 {
                    near |= bounds.width() == 3.0;
                    far |= bounds.width() == 7.0;
                }
            }
            _ => {}
        }
    }
    assert!(saw_transform && near && !far);
}

#[test]
fn a_singular_blurred_box_has_no_output() {
    let scene = paint_html(
        "<!doctype html><div style='width:37px;height:19px;
        background:red;box-shadow:1px 1px 4px black;transform:scale(0);filter:blur(5px)'></div>",
        1.0,
    )
    .unwrap();
    for command in scene.commands {
        match command {
            RenderCommand::PushLayer(layer) => assert!(layer.filter.is_none()),
            RenderCommand::BoxShadow(_) => panic!("singular shadow painted"),
            RenderCommand::Fill(fill) => assert_ne!(fill.shape.bounding_box().width(), 37.0),
            _ => {}
        }
    }
}

#[test]
fn a_box_beyond_the_source_clip_can_cast_a_shadow_into_it() {
    let scene = paint_html(
        "<!doctype html><style>body{margin:0}
        #f{position:absolute;left:220px;top:20px;width:20px;height:20px;filter:blur(4px)}
        #child{position:absolute;left:60px;width:6px;height:6px;box-shadow:-45px 0 2px red}
        </style><div id=f><div id=child></div></div>",
        1.0,
    )
    .unwrap();
    assert!(
        scene
            .commands
            .iter()
            .any(|cmd| matches!(cmd, RenderCommand::BoxShadow(shadow)
        if shadow.offset.x == -45.0))
    );
}

#[test]
fn an_offscreen_parent_does_not_cull_a_descendant_blur_touching_the_viewport() {
    let scene = paint_html(
        "<!doctype html><style>body{margin:0}
        #parent{position:absolute;left:245px;top:30px;width:6px;height:8px}
        #source{width:6px;height:8px;filter:blur(4px);background:red}
        </style><div id=parent><div id=source></div></div>",
        1.0,
    )
    .unwrap();
    assert!(scene.commands.iter().any(|command| matches!(command,
        RenderCommand::PushLayer(layer) if layer.filter.is_some())));
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

const TITLE_FONT: &[u8] = include_bytes!("../../../examples/wasm_hello/assets/DejaVuSans.woff2");

fn paint_title(css: &str, font: &[u8]) -> Scene {
    let html = format!(
        "<!doctype html><style>body{{margin:0}}
         #t{{position:absolute;left:95px;top:70px;width:8px;height:12px;
         padding:2.5px 3.5px;white-space:nowrap;font:32px/1 sans-serif;
         filter:blur(2px);{css}}}</style><div id=t><span>ff</span></div>"
    );
    paint_html_with_font(&html, 1.0, Some(font)).unwrap()
}

fn title_layer(scene: &Scene) -> (kurbo::Rect, kurbo::Affine) {
    scene
        .commands
        .iter()
        .find_map(|command| match command {
            RenderCommand::PushLayer(layer) => {
                Some((layer.filter.as_ref()?.output_region()?, layer.transform))
            }
            _ => None,
        })
        .expect("blur on title")
}

#[test]
fn static_and_synthetic_italic_titles_keep_the_glyph_ink_in_a_local_crop() {
    use kurbo::{Affine, Rect};
    use skrifa::{
        FontRef, GlyphId, MetadataProvider as _,
        instance::{LocationRef, Size},
    };

    for italic in [false, true] {
        let scene = paint_title(if italic { "font-style:italic" } else { "" }, TITLE_FONT);
        let (crop, layer_transform) = title_layer(&scene);
        let mut glyph_count = 0;
        for command in &scene.commands {
            let RenderCommand::GlyphRun(run) = command else {
                continue;
            };
            assert_eq!(run.glyph_transform.is_some(), italic);
            let font =
                FontRef::from_index(run.font_data.data.as_ref(), run.font_data.index).unwrap();
            let metrics = font.glyph_metrics(Size::new(run.font_size), LocationRef::default());
            for glyph in &run.glyphs {
                let b = metrics.bounds(GlyphId::new(glyph.id)).unwrap();
                let outline = Rect::new(
                    b.x_min.into(),
                    (-b.y_max).into(),
                    b.x_max.into(),
                    (-b.y_min).into(),
                );
                let transform = layer_transform.inverse()
                    * run.transform
                    * Affine::translate((f64::from(glyph.x), f64::from(glyph.y)))
                    * run.glyph_transform.unwrap_or(Affine::IDENTITY);
                // Independent per-glyph outline bounds, plus the one-device-pixel
                // hinting margin and the three-sigma blur halo (six pixels).
                let visible = transform.transform_rect_bbox(outline).inflate(7.0, 7.0);
                assert_eq!(crop.union(visible), crop, "{visible:?} outside {crop:?}");
                glyph_count += 1;
            }
        }
        assert!(glyph_count > 0);
        assert!(crop.width() < 175.0 && crop.height() < 145.0, "{crop:?}");
    }
}

#[test]
fn unmeasured_decorations_and_text_shadows_keep_the_viewport_crop() {
    for style in ["text-decoration:overline", "text-shadow:80px 0 2px red"] {
        let scene = paint_title(style, TITLE_FONT);
        let (crop, _) = title_layer(&scene);
        assert!(
            crop.width() >= 240.0 && crop.height() >= 200.0,
            "{style}: {crop:?}"
        );
    }
}
