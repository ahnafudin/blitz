//! Declared font-face lifecycle, from admitted response through registration and layout.

use crate::BaseDocument;
use blitz_traits::node_id::NodeId;
use cssparser::{Parser, Token};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontFaceLoadStatus {
    Loading,
    Registered,
    Loaded,
    Error,
}
impl FontFaceLoadStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Loading | Self::Registered => "loading",
            Self::Loaded => "loaded",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Debug)]
pub struct FontFaceLoad {
    pub id: usize,
    pub owner: Option<NodeId>,
    pub family: String,
    pub weight: f32,
    pub style: String,
    pub status: FontFaceLoadStatus,
}
/// One lock owns every font-discovery/readiness field and its revision.
#[derive(Default)]
pub struct FontLoadingState {
    faces: BTreeMap<usize, FontFaceLoad>,
    pending: usize,
    revision: u64,
    frozen: bool,
    invalid: bool,
}
impl std::ops::Deref for FontLoadingState {
    type Target = BTreeMap<usize, FontFaceLoad>;
    fn deref(&self) -> &Self::Target {
        &self.faces
    }
}
impl std::ops::DerefMut for FontLoadingState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.revision += 1;
        &mut self.faces
    }
}
pub type FontFaceLoads = Arc<Mutex<FontLoadingState>>;
pub struct FontStylesheetState {
    inner: FontFaceLoads,
    #[cfg(feature = "ahcut")]
    pub(crate) author_styles: crate::style_admission::StyleAdmission,
}
impl FontStylesheetState {
    pub(crate) fn new(inner: FontFaceLoads) -> Self {
        Self {
            inner,
            #[cfg(feature = "ahcut")]
            author_styles: Default::default(),
        }
    }
    pub(crate) fn active(&self) -> bool {
        !self.inner.lock().unwrap().invalid
    }
    #[cfg(feature = "ahcut")]
    pub(crate) fn pending(
        self: &Arc<Self>,
        shell: Arc<dyn blitz_traits::shell::ShellProvider>,
    ) -> PendingStylesheet {
        {
            let mut state = self.inner.lock().unwrap();
            state.pending += 1;
            state.revision += 1;
        }
        PendingStylesheet {
            state: self.clone(),
            shell,
        }
    }
}
#[cfg(feature = "ahcut")]
pub(crate) struct PendingStylesheet {
    state: Arc<FontStylesheetState>,
    shell: Arc<dyn blitz_traits::shell::ShellProvider>,
}
#[cfg(feature = "ahcut")]
impl Drop for PendingStylesheet {
    fn drop(&mut self) {
        {
            let mut state = self.state.inner.lock().unwrap();
            state.pending -= 1;
            state.revision += 1;
        }
        self.shell.request_redraw();
    }
}
pub struct FontLoadingSnapshot {
    pub faces: Vec<FontFaceLoad>,
    pub ids: Vec<usize>,
    pub waiter_ids: Vec<Vec<usize>>,
    pub blocked: bool,
    pub revision: u64,
}
impl BaseDocument {
    /// The diagnostic returned by the font snapshot after a frozen graph is invalidated.
    pub const INVALIDATED_FONT_GRAPH: &'static str = "STYLESHEET_REBUILD_REQUIRED:a caught style sheet change stopped font loading; declare style sheets in the initial HTML";

    pub fn freeze_font_stylesheets(&self) {
        let mut state = self.font_loads.lock().unwrap();
        state.frozen = true;
        state.revision += 1;
    }
    pub fn font_stylesheets_frozen(&self) -> bool {
        self.font_loads.lock().unwrap().frozen
    }
    pub fn font_stylesheets_valid(&self) -> bool {
        !self.font_loads.lock().unwrap().invalid
    }
    pub fn pending_font_stylesheets(&self) -> bool {
        self.font_loads.lock().unwrap().pending != 0
    }
    pub(crate) fn refuse_frozen_font_stylesheet_mutation(&self) -> bool {
        let mut state = self.font_loads.lock().unwrap();
        if state.frozen {
            state.invalid = true;
            state.revision += 1;
            true
        } else {
            false
        }
    }
    /// IDs, faces, pending and revision derive from one captured lock revision.
    pub fn font_loading_snapshot(
        &self,
        requested: Option<&str>,
        waiting: &[String],
    ) -> Result<FontLoadingSnapshot, String> {
        let (faces, pending, revision) = {
            let state = self.font_loads.lock().unwrap();
            if state.invalid {
                return Err(Self::INVALIDATED_FONT_GRAPH.into());
            }
            (
                state.faces.values().cloned().collect::<Vec<_>>(),
                state.pending,
                state.revision,
            )
        };
        let ids = requested
            .map(|font| self.matching_font_face_ids_in(font, &faces))
            .transpose()?
            .unwrap_or_default();
        let waiter_ids = waiting
            .iter()
            .map(|font| self.matching_font_face_ids_in(font, &faces))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(FontLoadingSnapshot {
            faces,
            ids,
            waiter_ids,
            blocked: pending != 0 || self.has_pending_critical_resources(),
            revision,
        })
    }
}

impl BaseDocument {
    /// Snapshot only after resolve has processed responses, registered fonts and laid out text.
    pub fn font_face_loads(&self) -> Vec<FontFaceLoad> {
        self.font_loads.lock().unwrap().values().cloned().collect()
    }

    pub(crate) fn finish_font_layout(&self) {
        for face in self.font_loads.lock().unwrap().values_mut() {
            if face.status == FontFaceLoadStatus::Registered {
                face.status = FontFaceLoadStatus::Loaded;
            }
        }
    }

    pub(crate) fn retire_font_faces_for_node(&mut self, owner: NodeId) {
        if self.refuse_frozen_font_stylesheet_mutation() {
            return;
        }
        let mut loads = self.font_loads.lock().unwrap();
        let retired: Vec<usize> = loads
            .values()
            .filter(|face| face.owner == Some(owner))
            .map(|face| face.id)
            .collect();
        loads.retain(|_, face| face.owner != Some(owner));
        drop(loads);
        // With `ahcut`, a late response for a retired face is dropped instead of registered.
        #[cfg(feature = "ahcut")]
        self.retired_font_loads.extend(retired);
        #[cfg(not(feature = "ahcut"))]
        let _ = retired;
    }

    /// Parse with the same CSS parser as element.style, then select declared variants.
    pub fn matching_font_face_ids(&self, font: &str) -> Result<Vec<usize>, String> {
        self.matching_font_face_ids_in(font, &self.font_face_loads())
    }

    fn matching_font_face_ids_in(
        &self,
        font: &str,
        faces: &[FontFaceLoad],
    ) -> Result<Vec<usize>, String> {
        let declarations = self
            .style_attr_set_property("", "font", font, false)
            .ok_or_else(|| "Invalid font shorthand".to_owned())?;
        let families = self.style_attr_get_property(&declarations, "font-family");
        if families.is_empty() {
            return Err("A font family is required".into());
        }
        let weight = self.style_attr_get_property(&declarations, "font-weight");
        let weight = match weight.as_str() {
            "normal" | "" => 400.0,
            "bold" => 700.0,
            value => value
                .parse::<f32>()
                .map_err(|_| "Unresolved font weight".to_owned())?,
        };
        let requested_style = self.style_attr_get_property(&declarations, "font-style");
        let requested_style = style_class(&requested_style);
        let mut parser = Parser::new(&families);
        let families: Vec<String> = parser
            .parse_comma_separated(|family| {
                let mut name = String::new();
                while !family.is_exhausted() {
                    match family.next()? {
                        Token::Ident(value) | Token::QuotedString(value) => {
                            if !name.is_empty() {
                                name.push(' ');
                            }
                            name.push_str(value);
                        }
                        _ => return Err(cssparser::ParseError::<()>::custom(())),
                    }
                }
                if name.is_empty() {
                    return Err(cssparser::ParseError::<()>::custom(()));
                }
                Ok(name)
            })
            .map_err(|_: cssparser::ParseError<()>| "Invalid font family list".to_owned())?;
        let mut ids = Vec::new();
        for family in families {
            let candidates: Vec<_> = faces
                .iter()
                .filter(|face| face.family.eq_ignore_ascii_case(&family))
                .collect();
            let Some(best) = candidates.iter().min_by(|a, b| {
                let style_rank = |face: &FontFaceLoad| {
                    let style = style_class(&face.style);
                    if style == requested_style {
                        0
                    } else if style == "normal" {
                        1
                    } else {
                        2
                    }
                };
                style_rank(a).cmp(&style_rank(b)).then_with(|| {
                    let a = weight_rank(weight, a.weight);
                    let b = weight_rank(weight, b.weight);
                    a.0.cmp(&b.0).then_with(|| a.1.total_cmp(&b.1))
                })
            }) else {
                continue;
            };
            ids.extend(
                candidates
                    .iter()
                    .filter(|face| {
                        face.weight == best.weight
                            && style_class(&face.style) == style_class(&best.style)
                    })
                    .map(|face| face.id),
            );
        }
        Ok(ids)
    }
}

/// A font family that an element with text of its own names first and that no registered font answers to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnregisteredFontFamily {
    pub node: NodeId,
    /// The element as its author would point at it: tag, id and classes, like `span#text.title`, and
    /// `::before` or `::after` for generated content.
    pub element: String,
    pub family: String,
}

/// An element as its author would point at it: tag, id and classes.
fn describe_element(element: &crate::ElementData) -> String {
    let mut described = element.name.local.to_string();
    if let Some(id) = element.attr(markup5ever::local_name!("id")) {
        described.push('#');
        described.push_str(id);
    }
    for class in element
        .attr(markup5ever::local_name!("class"))
        .unwrap_or("")
        .split_ascii_whitespace()
    {
        described.push('.');
        described.push_str(class);
    }
    described
}

impl BaseDocument {
    /// The first element in tree order that is rendered, has a text node as a child, and whose computed
    /// `font-family` list STARTS with a named family for which the font collection holds no font: the author's
    /// primary face is missing. Generated content (`::before`, `::after`) is an element here like any other.
    ///
    /// The rest of the list is resolved the way a browser without system fonts resolves it: a family that is not
    /// loaded is skipped, a generic family is whatever the collection maps it to, and each character is shaped
    /// by the first face in the list that has it. A generic family, or a name that is an alias of one on every build target (`-apple-system`,
    /// `BlinkMacSystemFont`, `-moz-fixed`), is not a family a document must load; every other name is, `-moz-` names included.
    pub fn first_unregistered_font_family(&self) -> Option<UnregisteredFontFamily> {
        use crate::stylo_to_parley::query_font_family;
        use parley::fontique::QueryFamily;

        let mut font_ctx = self.font_ctx.lock().unwrap();
        // Each entry: the node, and how to name it when it is generated content.
        let mut stack: Vec<(NodeId, Option<(String, &str)>)> = vec![(self.root_node().id, None)];
        while let Some((id, pseudo)) = stack.pop() {
            let Some(node) = self.get_node(id) else {
                continue;
            };
            let Some(element) = node.element_data() else {
                // The document node: its children are the tree. Text and comments have none.
                stack.extend(node.children.iter().rev().map(|child| (*child, None)));
                continue;
            };
            let Some(styles) = node.primary_styles() else {
                continue;
            };
            // Nothing below an element without a box is rendered.
            if styles.get_box().display.is_none() {
                continue;
            }
            let described = match &pseudo {
                Some((parent, which)) => format!("{parent}{which}"),
                None => describe_element(element),
            };
            // Generated content is a child box of its element, laid out before and after the children.
            if pseudo.is_none() {
                for (generated, which) in [(node.after(), "::after"), (node.before(), "::before")] {
                    if let Some(generated) = generated {
                        stack.push((generated, Some((described.clone(), which))));
                    }
                }
            }
            stack.extend(node.children.iter().rev().map(|child| (*child, None)));
            let has_text = node.children.iter().any(|child| {
                self.get_node(*child)
                    .and_then(|child| child.text_data())
                    .is_some_and(|text| !text.content.is_empty())
            });
            if !has_text {
                continue;
            }
            let Some(first) = styles.get_font().font_family.families.iter().next() else {
                continue;
            };
            if let QueryFamily::Named(name) = query_font_family(first)
                && font_ctx.collection.family_id(name).is_none()
            {
                return Some(UnregisteredFontFamily {
                    node: id,
                    element: described,
                    family: name.to_owned(),
                });
            }
        }
        None
    }
}

fn style_class(style: &str) -> &'static str {
    if style.starts_with("italic") {
        "italic"
    } else if style.starts_with("oblique") {
        "oblique"
    } else {
        "normal"
    }
}

fn weight_rank(requested: f32, candidate: f32) -> (u8, f32) {
    if (400.0..=500.0).contains(&requested) {
        if (requested..=500.0).contains(&candidate) {
            (0, candidate - requested)
        } else if candidate < requested {
            (1, requested - candidate)
        } else {
            (2, candidate - 500.0)
        }
    } else if requested < 400.0 {
        if candidate <= requested {
            (0, requested - candidate)
        } else {
            (1, candidate - requested)
        }
    } else if candidate >= requested {
        (0, candidate - requested)
    } else {
        (1, requested - candidate)
    }
}
