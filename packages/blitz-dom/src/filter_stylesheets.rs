//! Author-sheet provenance for the narrow SVG filter-color profile.

use crate::{BaseDocument, NodeId};
use anyrender::filters::FilterError;
use cssparser::{ParseError, Parser, Token};
use style::servo_arc::Arc as ServoArc;
use style::shared_lock::ToCssWithGuard;
use style::stylesheets::{DocumentStyleSheet, Origin, StylesheetInDocument};
use style_traits::CssStringWriter;

pub(crate) struct EmbeddedFilterSheet {
    sheet: DocumentStyleSheet,
    dom_source: String,
    parser_source: String,
    parsed_signature: Option<String>,
}
/// Refuses CSS that can give `color-interpolation-filters` a value: a declaration of that property or of `all`.
/// This Stylo pin does not compute the property, so the filter resolver reads the presentation attribute instead,
/// and that is only right while no CSS sets it. `declarations` says whether a declaration can stand at this level:
/// inside any `{}` block, and at the top of a style attribute. Elsewhere the two names are ordinary identifiers
/// (`@media all`, `transition: all`) and set nothing.
fn tokens<'i>(parser: &mut Parser<'i>, declarations: bool) -> Result<(), ParseError<()>> {
    // True where a declaration or a rule can begin: at the start, after `;` and after a `{}` block.
    let mut item_start = true;
    while !parser.is_exhausted() {
        let token = parser.next_including_whitespace_and_comments()?.clone();
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) => continue,
            Token::Semicolon => {
                item_start = true;
                continue;
            }
            Token::Ident(name)
                if declarations
                    && item_start
                    && (name.eq_ignore_ascii_case("color-interpolation-filters")
                        || name.eq_ignore_ascii_case("all")) =>
            {
                // A declaration only when a colon follows; otherwise it is a selector or a value.
                let state = parser.state();
                let declared = matches!(parser.next(), Ok(Token::Colon));
                parser.reset(&state);
                if declared {
                    return Err(ParseError::custom(()));
                }
            }
            Token::AtKeyword(name) if name.eq_ignore_ascii_case("import") => {
                return Err(ParseError::custom(()));
            }
            Token::CurlyBracketBlock => {
                parser.parse_nested_block(|block| tokens(block, true))?;
                item_start = true;
                continue;
            }
            Token::Function(_) | Token::SquareBracketBlock | Token::ParenthesisBlock => {
                parser.parse_nested_block(|block| tokens(block, false))?
            }
            Token::BadUrl(_) | Token::BadString(_) => return Err(ParseError::custom(())),
            _ => {}
        }
        item_start = false;
    }
    Ok(())
}
fn check_source(css: &str) -> Result<(), FilterError> {
    tokens(&mut Parser::new(css), false)
        .map_err(|_| FilterError::Unsupported("author stylesheet color/import provenance"))
}
/// A style attribute on the way from a filter primitive to the root: a declaration list, held to the same rule as
/// an author sheet, because the property inherits and an inline declaration outranks the presentation attribute.
pub(crate) fn check_inline_style(css: &str) -> Result<(), FilterError> {
    tokens(&mut Parser::new(css), true)
        .map_err(|_| FilterError::Unsupported("inline style sets the filter color space"))
}
impl BaseDocument {
    fn filter_sheet_signature(&self, sheet: &DocumentStyleSheet) -> Option<String> {
        let guard = self.guard.read();
        let contents = sheet.contents(&guard);
        let rules = contents.rules.read_with(&guard);
        let mut text = CssStringWriter::new();
        for rule in &rules.0 {
            rule.to_css(&guard, &mut text).ok()?;
        }
        Some(text)
    }
    pub(crate) fn remember_embedded_filter_sheet(
        &mut self,
        node: NodeId,
        sheet: &DocumentStyleSheet,
        css: &str,
    ) {
        let is_style = self
            .nodes
            .get(node)
            .and_then(|node| node.element_data())
            .is_some_and(|e| &*e.name.local == "style");
        if !is_style {
            self.filter_author_sources.remove(&node);
            return;
        }
        let known_nodes = &self.nodes_to_stylesheet;
        self.filter_author_sources
            .retain(|id, _| *id == node || known_nodes.contains_key(id));
        let record = EmbeddedFilterSheet {
            sheet: sheet.clone(),
            dom_source: self.nodes[node].text_content(),
            parser_source: css.to_owned(),
            parsed_signature: self.filter_sheet_signature(sheet),
        };
        self.filter_author_sources.insert(node, record);
    }
    pub(crate) fn invalidate_filter_stylesheet_provenance(&mut self, node: NodeId) {
        self.filter_author_sources.remove(&node);
    }
    /// Fail closed for active linked, imported, supplied or mutated author CSS.
    /// Only unchanged embedded sources observed at their real parser entry are admitted.
    pub fn validate_filter_stylesheet_provenance(&self) -> Result<(), FilterError> {
        let mut pending = vec![self.root_node().id];
        while let Some(id) = pending.pop() {
            let node = &self.nodes[id];
            if let Some(element) = node.element_data() {
                let is_stylesheet_link = &*element.name.local == "link"
                    && element.attrs.iter().any(|a| {
                        &*a.name.local == "rel"
                            && a.value
                                .split_ascii_whitespace()
                                .any(|v| v.eq_ignore_ascii_case("stylesheet"))
                    });
                if is_stylesheet_link {
                    return Err(FilterError::Unsupported(
                        "linked author stylesheet provenance is unestablished",
                    ));
                }
            }
            pending.extend(node.children.iter().copied());
        }
        for i in 0..self.stylist.sheet_count(Origin::Author) {
            let sheet = self
                .stylist
                .sheet_at(Origin::Author, i)
                .ok_or(FilterError::Unsupported("author stylesheet enumeration"))?;
            let (owner, _) = self
                .nodes_to_stylesheet
                .iter()
                .find(|(_, s)| ServoArc::ptr_eq(&s.0, &sheet.0))
                .ok_or(FilterError::Unsupported(
                    "author stylesheet provenance is unestablished",
                ))?;
            let source = self
                .filter_author_sources
                .get(owner)
                .ok_or(FilterError::Unsupported(
                    "author stylesheet provenance is unestablished",
                ))?;
            let node = self
                .nodes
                .get(*owner)
                .ok_or(FilterError::Unsupported("author stylesheet owner is stale"))?;
            if !ServoArc::ptr_eq(&source.sheet.0, &sheet.0)
                || node.text_content() != source.dom_source
                || self.filter_sheet_signature(sheet) != source.parsed_signature
                || source.parsed_signature.is_none()
            {
                return Err(FilterError::Unsupported(
                    "author stylesheet provenance is unestablished",
                ));
            }
            check_source(&source.parser_source)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_decoded_color_names_imports_and_all_at_any_depth() {
        for css in [
            "filter{color-interpolation-filters:sRGB}",
            r"filter{c\6flor-interpolation-filters:sRGB}",
            "@media screen{filter{color-interpolation-filters:sRGB}}",
            "@import url(child.css);",
            "filter{all:inherit}",
        ] {
            assert!(check_source(css).is_err(), "{css}");
        }
        assert!(
            check_source("html,body{margin:0}#subject{background:red;filter:url(#mb)}").is_ok()
        );
    }

    #[test]
    fn only_declarations_of_the_two_names_are_refused() {
        for css in [
            "@media all{a{transition:all 1s}}",
            "a{transition-property:all}",
            "all{color:red}",
            "@keyframes all{from{left:0}to{left:1px}}",
        ] {
            assert!(check_source(css).is_ok(), "{css}");
        }
        for css in [
            "a{left:0;all:unset}",
            "a{b{}all:unset}",
            "a{ALL : revert}",
            "@media all{a{/*x*/all/**/:initial}}",
        ] {
            assert!(check_source(css).is_err(), "{css}");
        }
        for css in [
            "transform:none;left:0;top:0",
            "transition:all 1s",
            "--x:all",
        ] {
            assert!(check_inline_style(css).is_ok(), "{css}");
        }
        for css in [
            "color-interpolation-filters:sRGB",
            "left:0;COLOR-INTERPOLATION-FILTERS : linearRGB",
            r"c\6flor-interpolation-filters:sRGB",
            "all:unset",
            "left:0;/*x*/all/**/:initial",
            "background:url(bad value)",
        ] {
            assert!(check_inline_style(css).is_err(), "{css}");
        }
    }

    #[test]
    fn escaped_urls_keep_token_validation_and_import_refusals() {
        for css in [
            r"#subject{filter:u\72l(#mb)}",
            r##"#subject{filter:url("#mb")}"##,
            "filter{content:\"color-interpolation-filters\"}",
        ] {
            assert!(check_source(css).is_ok(), "{css}");
        }
        for css in [
            r"@\69mport u\72l(child.css);",
            r"@media screen{filter{c\6flor-interpolation-filters:sRGB}}",
            r"filter{background:u\72l(bad value)}",
            "filter{content:\"bad\nstring\"}",
        ] {
            assert!(check_source(css).is_err(), "{css}");
        }
    }
}
