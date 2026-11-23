//! The `color-interpolation-filters` declaration of a style attribute, carried through CSSOM writes (`ahcut`).

use cssparser::{Delimiter, ParseError, Parser, Token};

/// `color-interpolation-filters` is not a property of this Stylo build, so Stylo drops it whenever it re-serializes a
/// style attribute. A browser applies it, and the SVG filter colour rule must see it: a style attribute that declares
/// it refuses every filter whose primitive it can reach. So the CSSOM writes of a style attribute (`setProperty`,
/// `removeProperty`, `cssText`, and the property accessors that call them) carry its last declaration over.
pub(super) const FILTER_COLOR_SPACE: &str = "color-interpolation-filters";

/// A CSS keyword the declaration can carry: letters, digits and dashes only, so it serializes as written.
fn is_keyword(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// A declaration value as it may be carried: a keyword, or a value that holds a function CSS substitutes only at
/// computed-value time (see [`is_substitution`]), which is never resolved here. Either one is written back token
/// by token, so the text cannot close the declaration early or open a block at its top level. Returns the value
/// and whether it ended in `!important`.
pub(super) fn carried_value(value: &str) -> Option<(String, bool)> {
    let mut parser = Parser::new(value);
    let mut written = SerializedValue::default();
    serialize_value(&mut parser, &mut written, Level::Declaration).ok()?;
    let text = written.text.trim().to_owned();
    (is_keyword(&text) || written.substituted).then_some((text, written.important))
}

#[derive(Default)]
struct SerializedValue {
    text: String,
    /// It holds a substitution function at some depth.
    substituted: bool,
    /// It ended in `!important` at the top level, which is not part of `text`.
    important: bool,
}

/// The functions CSS substitutes at computed-value time, so that a declaration holding one is valid when it is set
/// whatever it resolves to: `var()`, `env()`, `attr()`, `if()`, `inherit()` and a custom function (`--name()`).
fn is_substitution(name: &str) -> bool {
    ["var", "env", "attr", "if", "inherit"]
        .iter()
        .any(|known| name.eq_ignore_ascii_case(known))
        || name.starts_with("--")
}

/// Where a token of a declaration value stands.
#[derive(Clone, Copy, PartialEq)]
enum Level {
    /// Directly in the declaration.
    Declaration,
    /// Inside a function or bracket.
    Nested,
    /// Directly inside `if()`, whose branches end in `;`.
    Condition,
}

/// Writes the tokens of one declaration value. Refuses what cannot stand in one: a `;` anywhere but between the
/// branches of `if()`, a `{}` block directly in the declaration, an unmatched closing bracket, a bad string or URL,
/// and a `!` other than a final `!important` directly in the declaration.
fn serialize_value(
    parser: &mut Parser<'_>,
    written: &mut SerializedValue,
    level: Level,
) -> Result<(), ParseError<()>> {
    while let Ok(token) = parser.next_including_whitespace_and_comments() {
        let token = token.clone();
        match &token {
            Token::Semicolon if level != Level::Condition => return Err(ParseError::custom(())),
            Token::CurlyBracketBlock if level == Level::Declaration => {
                return Err(ParseError::custom(()));
            }
            Token::CloseCurlyBracket
            | Token::CloseParenthesis
            | Token::CloseSquareBracket
            | Token::BadUrl(_)
            | Token::BadString(_) => return Err(ParseError::custom(())),
            Token::Delim('!') => {
                let important = matches!(parser.next(), Ok(Token::Ident(word)) if word.eq_ignore_ascii_case("important"));
                if level != Level::Declaration || !important || parser.next().is_ok() {
                    return Err(ParseError::custom(()));
                }
                written.important = true;
            }
            // A comment still separates the tokens around it.
            Token::Comment(_) => written.text.push(' '),
            Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => {
                let mut inside = Level::Nested;
                if let Token::Function(name) = &token
                    && is_substitution(name)
                {
                    written.substituted = true;
                    if name.eq_ignore_ascii_case("if") {
                        inside = Level::Condition;
                    }
                }
                cssparser::ToCss::to_css(&token, &mut written.text)
                    .map_err(|_| ParseError::custom(()))?;
                parser.parse_nested_block(|block| serialize_value(block, written, inside))?;
                written.text.push(match token {
                    Token::SquareBracketBlock => ']',
                    Token::CurlyBracketBlock => '}',
                    _ => ')',
                });
            }
            _ => cssparser::ToCss::to_css(&token, &mut written.text)
                .map_err(|_| ParseError::custom(()))?,
        }
    }
    Ok(())
}

/// The last declaration of `color-interpolation-filters` in a style attribute whose value can be carried (see
/// [`carried_value`]), with its `!important`. Escapes, letter case and comments are read as CSS reads them.
pub(super) fn filter_color_space_declaration(style_attr: &str) -> Option<(String, bool)> {
    let mut parser = Parser::new(style_attr);
    let mut found = None;
    while !parser.is_exhausted() {
        let declaration = parser.parse_until_after(Delimiter::Semicolon, |p| {
            let named = matches!(p.next(), Ok(Token::Ident(name)) if name.eq_ignore_ascii_case(FILTER_COLOR_SPACE));
            if !named || !matches!(p.next(), Ok(Token::Colon)) {
                return Ok::<_, ParseError<()>>(None);
            }
            let start = p.position();
            while p.next_including_whitespace_and_comments().is_ok() {}
            Ok(carried_value(p.slice_from(start)))
        });
        if let Ok(Some(carried)) = declaration {
            found = Some(carried);
        }
    }
    found
}

/// `css` (a serialized declaration list) with the carried declaration appended, if there is one.
pub(super) fn with_filter_color_space(css: String, declaration: Option<(String, bool)>) -> String {
    let Some((value, important)) = declaration else {
        return css;
    };
    let separator = if css.is_empty() {
        ""
    } else if css.ends_with(';') {
        " "
    } else {
        "; "
    };
    let important = if important { " !important" } else { "" };
    format!("{css}{separator}{FILTER_COLOR_SPACE}: {value}{important};")
}
