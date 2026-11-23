//! Inspect author CSS before Stylo discards unsupported declarations. Keep the first refusal per document.

use crate::BaseDocument;
use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, Token,
};
use std::sync::OnceLock;

const KEYFRAMES: &str = "CSS_KEYFRAMES_REFUSED:write the animation on the timeline in __seek";
const TEXT_STROKE: &str = "STYLE_PROPERTY_REFUSED:use filled text or an outlined SVG path";

#[derive(Clone, Copy)]
pub enum StyleInput {
    Sheet,
    Declarations,
    Property,
}

#[derive(Default)]
pub(crate) struct StyleAdmission(OnceLock<&'static str>);

impl BaseDocument {
    pub fn script_style_refusal(&self) -> Option<&'static str> {
        self.font_stylesheets.author_styles.0.get().copied()
    }

    pub fn admit_script_style(&self, input: StyleInput, text: &str) -> Result<(), &'static str> {
        self.font_stylesheets.author_styles.check(input, text)
    }
}

impl StyleAdmission {
    /// Both network stylesheet paths pass no protocol or environment encoding to Stylo.
    pub(crate) fn check_bytes(&self, bytes: &[u8]) {
        let encoding = cssparser::stylesheet_encoding::<CssEncoding>(bytes, None, None);
        let (css, _, _) = encoding.decode(bytes);
        let _ = self.check(StyleInput::Sheet, &css);
    }

    pub(crate) fn check(&self, input: StyleInput, text: &str) -> Result<(), &'static str> {
        let refusal = match input {
            StyleInput::Property => stroke(text),
            StyleInput::Sheet => StyleSheetParser::new(&mut Parser::new(text), &mut Rules)
                .filter_map(Result::ok)
                .flatten()
                .next(),
            StyleInput::Declarations => body(&mut Parser::new(text)),
        };
        match refusal {
            Some(reason) => Err(*self.0.get_or_init(|| reason)),
            None => Ok(()),
        }
    }
}

struct CssEncoding;
impl cssparser::EncodingSupport for CssEncoding {
    type Encoding = &'static encoding_rs::Encoding;
    fn from_label(label: &[u8]) -> Option<Self::Encoding> {
        encoding_rs::Encoding::for_label(label)
    }
    fn utf8() -> Self::Encoding {
        encoding_rs::UTF_8
    }
    fn is_utf16_be_or_le(value: &Self::Encoding) -> bool {
        *value == encoding_rs::UTF_16LE || *value == encoding_rs::UTF_16BE
    }
}

fn stroke(name: &str) -> Option<&'static str> {
    [
        "-webkit-text-stroke",
        "-webkit-text-stroke-width",
        "-webkit-text-stroke-color",
    ]
    .iter()
    .any(|p| name.eq_ignore_ascii_case(p))
    .then_some(TEXT_STROKE)
}

struct Rules;
type Refusal = Option<&'static str>;
type Parsed<T> = Result<T, ParseError<()>>;

fn body(input: &mut Parser) -> Refusal {
    RuleBodyParser::new(input, &mut Rules)
        .filter_map(Result::ok)
        .flatten()
        .next()
}

fn skip(input: &mut Parser) {
    while input.next().is_ok() {}
}

impl<'i> AtRuleParser<'i> for Rules {
    type Prelude = bool;
    type AtRule = Refusal;
    type Error = ();
    fn parse_prelude(&mut self, name: CowRcStr<'i>, input: &mut Parser<'i>) -> Parsed<bool> {
        skip(input);
        Ok(
            name.eq_ignore_ascii_case("keyframes")
                || name.eq_ignore_ascii_case("-webkit-keyframes"),
        )
    }
    fn parse_block(
        &mut self,
        keyframes: bool,
        _: &ParserState,
        input: &mut Parser<'i>,
    ) -> Parsed<Refusal> {
        let result = if keyframes {
            Some(KEYFRAMES)
        } else {
            body(input)
        };
        skip(input);
        Ok(result)
    }
}

impl<'i> QualifiedRuleParser<'i> for Rules {
    type Prelude = ();
    type QualifiedRule = Refusal;
    type Error = ();
    fn parse_prelude(&mut self, input: &mut Parser<'i>) -> Parsed<()> {
        skip(input);
        Ok(())
    }
    fn parse_block(&mut self, _: (), _: &ParserState, input: &mut Parser<'i>) -> Parsed<Refusal> {
        let result = body(input);
        skip(input);
        Ok(result)
    }
}

impl<'i> DeclarationParser<'i> for Rules {
    type Declaration = Refusal;
    type Error = ();
    fn parse_value(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _: &ParserState,
    ) -> Parsed<Refusal> {
        // An identifier followed by a colon can also start a nested selector, e.g. a:hover{}.
        // A custom property can contain arbitrary blocks; its tokens are values, never rules.
        while let Ok(token) = input.next() {
            if matches!(token, Token::CurlyBracketBlock) && !name.starts_with("--") {
                return Err(ParseError::unexpected_token());
            }
        }
        Ok(stroke(&name))
    }
}

impl<'i> RuleBodyItemParser<'i, Refusal, ()> for Rules {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        true
    }
}
