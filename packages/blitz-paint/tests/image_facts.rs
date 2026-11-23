//! Host image facts participate in layout and resource completion before native raster paint.
#![cfg(feature = "ahcut")]

use anyrender::Scene;
use blitz_dom::{DocumentConfig, node::{ImageData, Status}};
use blitz_html::HtmlDocument;
use blitz_traits::{net::{ImageFacts, NetHandler, NetProvider, Request, Url}, shell::Viewport};
use std::sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}};

const FACTS: ImageFacts = ImageFacts { asset: 7, width: 20, height: 60, has_alpha: true };

#[derive(Default)]
struct Pictures {
    lookups: Mutex<Vec<String>>,
    bytes: AtomicUsize,
}

impl NetProvider for Pictures {
    fn image_facts(&self, _: usize, url: &Url) -> Result<Option<ImageFacts>, String> {
        self.lookups.lock().unwrap().push(url.path().into());
        Ok(Some(FACTS))
    }

    fn fetch(&self, _: usize, request: Request, handler: Box<dyn NetHandler>) {
        self.bytes.fetch_add(1, Ordering::Relaxed);
        handler.error(request.url.into(), "unexpected byte ask".into());
    }
}

#[test]
fn one_lookup_sizes_images_and_refuses_only_when_the_facts_are_painted() {
    let pictures = Arc::new(Pictures::default());
    let config = DocumentConfig {
        base_url: Some("https://assets.invalid/".into()),
        viewport: Some(Viewport { window_size: (200,200), ..Viewport::default() }),
        net_provider: Some(pictures.clone()),
        ..DocumentConfig::default()
    };
    let mut doc = HtmlDocument::from_html(r#"<!doctype html><style>
        body{margin:0;background:white} img{display:block} #small{width:10px}
        #bg{width:12px;height:12px;background-image:url(a.png)}
        </style><img id="natural" src="a.png"><img id="small" src="a.png"><div id="bg"></div>"#, config);
    doc.resolve(0.0);
    doc.handle_messages();
    doc.resolve(0.0);
    let natural = doc.get_element_by_id("natural").unwrap();
    let small = doc.get_element_by_id("small").unwrap();
    let bg = doc.get_element_by_id("bg").unwrap();
    for (id, width, height, y) in [(natural,20.,60.,0.),(small,10.,30.,60.),(bg,12.,12.,90.)] {
        let layout = doc.get_node(id).unwrap().final_layout();
        assert_eq!((layout.size.width, layout.size.height, layout.location.y), (width,height,y));
    }
    for id in [natural,small] {
        let data = doc.get_node(id).unwrap().element_data().unwrap().image_data();
        assert!(matches!(data,Some(ImageData::Facts(facts)) if *facts == FACTS));
    }
    let layer = doc.get_node(bg).unwrap().element_data().unwrap().background_images[0].as_ref().unwrap();
    assert_eq!(layer.status,Status::Ok);
    assert!(matches!(&layer.image,ImageData::Facts(facts) if *facts == FACTS));
    assert_eq!(*pictures.lookups.lock().unwrap(), ["/a.png"]);
    assert_eq!(pictures.bytes.load(Ordering::Relaxed),0);

    // Each remaining visible use refuses independently, even with a body background before it.
    let mut scene = Scene::new();
    for id in [natural,small,bg] {
        let refusal = blitz_paint::try_paint_scene(&mut scene,&mut doc,1.,200,200,0,0).unwrap_err();
        assert_eq!(refusal.to_string(),"IMAGE_REFUSED:write an SVG shape or omit the raster image until native image paint is available");
        assert!(scene.commands.is_empty());
        doc.mutate().set_style_property(id,"display","none");
        doc.resolve(0.);
    }
    blitz_paint::try_paint_scene(&mut scene,&mut doc,1.,200,200,0,0).unwrap();
    assert!(!scene.commands.is_empty());
}
