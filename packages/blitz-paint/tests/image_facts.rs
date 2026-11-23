//! Host image facts participate in layout and resource completion before native raster paint.
#![cfg(feature = "ahcut")]

use anyrender::Scene;
use blitz_dom::{
    BaseDocument, DocumentConfig, ImageSelectionStatus, NodeId, QualName, local_name,
    node::{ImageData, Status},
    ns,
};
use blitz_html::HtmlDocument;
use blitz_traits::{
    net::{ImageFacts, NetHandler, NetProvider, Request, Url},
    shell::Viewport,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

const FACTS: ImageFacts = ImageFacts {
    asset: 7,
    width: 20,
    height: 60,
    has_alpha: true,
};

#[derive(Default)]
struct Pictures {
    lookups: Mutex<Vec<String>>,
    bytes: AtomicUsize,
}

impl NetProvider for Pictures {
    fn image_facts(&self, _: usize, url: &Url) -> Result<Option<ImageFacts>, String> {
        self.lookups.lock().unwrap().push(url.path().into());
        match url.path() {
            "/missing" => Err("ASSET_MISSING:choose a project image".into()),
            "/zero" => Ok(Some(ImageFacts { width: 0, ..FACTS })),
            "/fallback.svg" | "/fallback.png" | "/bad-bytes" => Ok(None),
            _ => Ok(Some(FACTS)),
        }
    }

    fn fetch(&self, _: usize, request: Request, handler: Box<dyn NetHandler>) {
        self.bytes.fetch_add(1, Ordering::Relaxed);
        if request.url.path() == "/fallback.svg" {
            handler.bytes(
                request.url.into(),
                blitz_traits::net::Bytes::from_static(
                    br#"<svg xmlns="http://www.w3.org/2000/svg" width="5" height="8"/>"#,
                ),
            );
        } else if request.url.path() == "/fallback.png" {
            let mut png = std::io::Cursor::new(Vec::new());
            image::RgbaImage::from_pixel(2, 3, image::Rgba([17, 23, 42, 255]))
                .write_to(&mut png, image::ImageFormat::Png)
                .unwrap();
            handler.bytes(request.url.into(), png.into_inner().into());
        } else {
            handler.error(request.url.into(), "byte request failed".into());
        }
    }
}

#[test]
fn selected_image_completion_covers_reselection_cache_failure_and_detached_elements() {
    let pictures = Arc::new(Pictures::default());
    let mut doc = HtmlDocument::from_html(
        "<!doctype html><body><img id=a><img id=b></body>",
        DocumentConfig {
            base_url: Some("https://assets.invalid/".into()),
            net_provider: Some(pictures.clone()),
            ..DocumentConfig::default()
        },
    );
    let a = doc.get_element_by_id("a").unwrap();
    let b = doc.get_element_by_id("b").unwrap();
    let detached = doc
        .mutate()
        .create_element(QualName::new(None, ns!(html), local_name!("img")), vec![]);
    let src = QualName::new(None, ns!(), local_name!("src"));
    let select = |doc: &mut BaseDocument, id: NodeId, url: &str| {
        doc.mutate().set_attribute(id, src.clone(), url);
        doc.image_selection(id).unwrap().token
    };
    assert!(doc.image_selection(a).is_none());
    select(&mut doc, a, "missing");
    let latest = select(&mut doc, a, "shared.png");
    let peer = select(&mut doc, b, "shared.png");
    let detached_token = select(&mut doc, detached, "shared.png");
    assert_eq!(
        doc.image_selection(a).unwrap().status,
        ImageSelectionStatus::Pending
    );
    assert!(doc.pop_image_completion().is_none());
    doc.handle_messages();
    assert_eq!(doc.pop_image_completion(), Some((a, latest)));
    // Replace an already queued completion before its callback can run.
    let failed = select(&mut doc, b, "zero");
    assert_ne!(peer, failed);
    assert_eq!(
        doc.get_node(b)
            .unwrap()
            .element_data()
            .unwrap()
            .image_data()
            .and_then(ImageData::intrinsic_raster_size),
        Some((20, 60)),
        "the current image remains available until the replacement completes"
    );
    assert_eq!(doc.pop_image_completion(), Some((detached, detached_token)));
    assert!(doc.pop_image_completion().is_none());
    doc.handle_messages();
    assert_eq!(doc.pop_image_completion(), Some((b, failed)));
    assert!(
        matches!(&doc.image_selection(b).unwrap().status,ImageSelectionStatus::Failed(error) if error.starts_with("IMAGE_REFUSED:"))
    );
    assert!(
        doc.get_node(b)
            .unwrap()
            .element_data()
            .unwrap()
            .image_data()
            .is_none()
    );

    let cached = select(&mut doc, b, "shared.png");
    assert_eq!(
        doc.image_selection(b).unwrap().status,
        ImageSelectionStatus::Loaded
    );
    assert_eq!(doc.pop_image_completion(), Some((b, cached)));
    assert!(doc.pop_image_completion().is_none());
    assert_eq!(
        *pictures.lookups.lock().unwrap(),
        ["/missing", "/shared.png", "/zero"]
    );
    assert_eq!(pictures.bytes.load(Ordering::Relaxed), 0);
    let cleared = select(&mut doc, b, "shared.png");
    assert_ne!(cleared, cached);
    doc.mutate().clear_attribute(b, src.clone());
    assert!(doc.image_selection(b).is_none());
    assert!(doc.pop_image_completion().is_none());

    for (url, reason) in [
        ("missing", "ASSET_MISSING:choose a project image"),
        ("bad-bytes", "byte request failed"),
    ] {
        let token = select(&mut doc, b, url);
        doc.handle_messages();
        assert_eq!(doc.pop_image_completion(), Some((b, token)));
        assert_eq!(
            doc.image_selection(b).unwrap().status,
            ImageSelectionStatus::Failed(reason.into())
        );
    }
    let svg = select(&mut doc, b, "fallback.svg");
    doc.handle_messages();
    assert_eq!(doc.pop_image_completion(), Some((b, svg)));
    assert!(matches!(
        doc.get_node(b)
            .unwrap()
            .element_data()
            .unwrap()
            .image_data(),
        Some(ImageData::Svg(_))
    ));
    let raster = select(&mut doc, b, "fallback.png");
    doc.handle_messages();
    assert_eq!(doc.pop_image_completion(), Some((b, raster)));
    let image = doc
        .get_node(b)
        .unwrap()
        .element_data()
        .unwrap()
        .image_data()
        .unwrap();
    assert!(matches!(image, ImageData::Raster(_)));
    assert_eq!(image.intrinsic_raster_size(), Some((2, 3)));
    assert_eq!(pictures.bytes.load(Ordering::Relaxed), 3);
}

#[test]
fn one_lookup_sizes_images_and_refuses_only_when_the_facts_are_painted() {
    let pictures = Arc::new(Pictures::default());
    let config = DocumentConfig {
        base_url: Some("https://assets.invalid/".into()),
        viewport: Some(Viewport {
            window_size: (200, 200),
            ..Viewport::default()
        }),
        net_provider: Some(pictures.clone()),
        ..DocumentConfig::default()
    };
    let mut doc = HtmlDocument::from_html(
        r#"<!doctype html><style>
        body{margin:0;background:white} img{display:block} #small{width:10px}
        #bg{width:12px;height:12px;background-image:url(a.png)}
        </style><img id="natural" src="a.png"><img id="small" src="a.png"><div id="bg"></div>"#,
        config,
    );
    doc.resolve(0.0);
    doc.handle_messages();
    doc.resolve(0.0);
    let natural = doc.get_element_by_id("natural").unwrap();
    let small = doc.get_element_by_id("small").unwrap();
    let bg = doc.get_element_by_id("bg").unwrap();
    for (id, width, height, y) in [
        (natural, 20., 60., 0.),
        (small, 10., 30., 60.),
        (bg, 12., 12., 90.),
    ] {
        let layout = doc.get_node(id).unwrap().final_layout();
        assert_eq!(
            (layout.size.width, layout.size.height, layout.location.y),
            (width, height, y)
        );
    }
    for id in [natural, small] {
        let data = doc
            .get_node(id)
            .unwrap()
            .element_data()
            .unwrap()
            .image_data();
        assert!(matches!(data,Some(ImageData::Facts(facts)) if *facts == FACTS));
    }
    let layer = doc
        .get_node(bg)
        .unwrap()
        .element_data()
        .unwrap()
        .background_images[0]
        .as_ref()
        .unwrap();
    assert_eq!(layer.status, Status::Ok);
    assert!(matches!(&layer.image,ImageData::Facts(facts) if *facts == FACTS));
    assert_eq!(*pictures.lookups.lock().unwrap(), ["/a.png"]);
    assert_eq!(pictures.bytes.load(Ordering::Relaxed), 0);

    // Each remaining visible use refuses independently, even with a body background before it.
    let mut scene = Scene::new();
    for id in [natural, small, bg] {
        let refusal =
            blitz_paint::try_paint_scene(&mut scene, &mut doc, 1., 200, 200, 0, 0).unwrap_err();
        assert_eq!(
            refusal,
            anyrender::filters::FilterError::Unsupported(
                "IMAGE_PAINT_PENDING:draw the shape in SVG or omit the image until image paint lands"
            )
        );
        assert!(scene.commands.is_empty());
        doc.mutate().set_style_property(id, "display", "none");
        doc.resolve(0.);
    }
    blitz_paint::try_paint_scene(&mut scene, &mut doc, 1., 200, 200, 0, 0).unwrap();
    assert!(!scene.commands.is_empty());
    let body = doc.query_selector("body").unwrap().unwrap();
    for id in [natural, small, bg] {
        doc.mutate().remove_node(id);
    }
    assert!(
        !doc.has_image_facts(),
        "cached URLs and detached nodes do not require a paint preflight"
    );
    doc.mutate().append_children(body, &[small]);
    assert!(
        doc.has_image_facts(),
        "reattaching a cached image restores the preflight"
    );
    doc.mutate()
        .clear_attribute(small, QualName::new(None, ns!(), local_name!("src")));
    assert!(!doc.has_image_facts());
}
