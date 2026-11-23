//! Current HTML image requests and their completion tasks, including detached and cached images.

use crate::{
    BaseDocument, NodeId,
    layout::damage::ALL_DAMAGE,
    node::{ImageData, SpecialElementData, Status},
    util::ImageType,
};
use markup5ever::local_name;
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageSelectionStatus {
    Pending,
    Loaded,
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct ImageSelection {
    pub token: u64,
    pub url: Option<Url>,
    pub status: ImageSelectionStatus,
}

impl BaseDocument {
    pub fn has_image_facts(&self) -> bool {
        self.has_image_facts
    }

    pub fn image_selection(&self, id: NodeId) -> Option<&ImageSelection> {
        self.get_node(id)?
            .element_data()?
            .image_selection
            .as_deref()
    }

    /// Take one still-current completion. A handler may reselect another image before the next call.
    pub fn pop_image_completion(&mut self) -> Option<(NodeId, u64)> {
        while let Some((id, token)) = self.image_completions.pop_front() {
            if self
                .image_selection(id)
                .is_some_and(|selection| selection.token == token)
            {
                return Some((id, token));
            }
        }
        None
    }

    pub(crate) fn select_html_image(&mut self, id: NodeId) {
        let node = &self.nodes[id];
        let src = node.element_data().unwrap().attr(local_name!("src"));
        let selection = src.map(|value| {
            let url = if value.trim().is_empty() {
                None
            } else {
                self.resolve_url(value)
            };
            ImageSelection {
                token: self.image_selection_token,
                url,
                status: ImageSelectionStatus::Pending,
            }
        });
        self.image_selection_token = self
            .image_selection_token
            .checked_add(1)
            .expect("image selection counter exhausted");
        let url = selection
            .as_ref()
            .and_then(|selection| selection.url.clone());
        let node = &mut self.nodes[id];
        let element = node.element_data_mut().unwrap();
        element.image_selection = selection.map(Box::new);
        element.special_data = SpecialElementData::None;
        node.clear_layout_cache();
        node.insert_damage(ALL_DAMAGE);
        let Some(url) = url else {
            self.complete_image_selection(
                id,
                None,
                ImageSelectionStatus::Failed("IMAGE_REFUSED:use a valid image src".into()),
            );
            return;
        };
        if let Some(image) = self.image_cache.get(url.as_str()).cloned() {
            self.nodes[id].element_data_mut().unwrap().special_data =
                SpecialElementData::Image(Box::new(image));
            self.complete_image_selection(id, Some(url.as_str()), ImageSelectionStatus::Loaded);
        } else {
            self.fetch_image(url, id, ImageType::Image);
        }
    }

    pub(crate) fn complete_image_selection(
        &mut self,
        id: NodeId,
        url: Option<&str>,
        status: ImageSelectionStatus,
    ) -> bool {
        let Some(selection) = self
            .nodes
            .get_mut(id)
            .and_then(|n| n.element_data_mut())
            .and_then(|el| el.image_selection.as_deref_mut())
        else {
            return false;
        };
        if selection.status != ImageSelectionStatus::Pending
            || selection.url.as_ref().map(Url::as_str) != url
        {
            return false;
        }
        selection.status = status;
        self.image_completions.push_back((id, selection.token));
        true
    }

    pub(crate) fn fail_image_waiter(
        &mut self,
        id: NodeId,
        kind: ImageType,
        url: &str,
        error: &str,
    ) {
        let Some(element) = self.nodes.get_mut(id).and_then(|n| n.element_data_mut()) else {
            return;
        };
        let layer = match kind {
            ImageType::Image => {
                self.complete_image_selection(
                    id,
                    Some(url),
                    ImageSelectionStatus::Failed(error.to_owned()),
                );
                return;
            }
            ImageType::Background(idx) => element.background_images.get_mut(idx),
            ImageType::Mask(idx) => element.mask_images.get_mut(idx),
        };
        if let Some(Some(layer)) = layer
            && layer.url.as_str() == url
        {
            layer.status = Status::Error;
            layer.image = ImageData::None;
        }
    }
}
