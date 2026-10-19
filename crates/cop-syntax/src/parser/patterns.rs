use std::ops::Range;

use crate::{
    ast::{PathSyntax, PatternFieldSyntax, PatternSyntax, PatternSyntaxKind},
    Delimiter, Token, TokenKind,
};

use super::{
    path::PathParser,
    token_cursor::{SyntaxError, SyntaxErrorKind, TokenCursor},
};

pub(super) struct PatternParser;

impl PatternParser {
    pub(super) fn parse_prefix(
        cursor: &mut TokenCursor<'_>,
        bind_bare_identifiers: bool,
    ) -> Result<PatternSyntax, SyntaxError> {
        Self::parse(cursor, bind_bare_identifiers)
    }

    pub(super) fn parse_range(
        tokens: &[Token],
        bounds: Range<usize>,
    ) -> Result<PatternSyntax, SyntaxError> {
        Self::parse_range_with_binding_mode(tokens, bounds, false)
    }

    pub(super) fn parse_binding_range(
        tokens: &[Token],
        bounds: Range<usize>,
    ) -> Result<PatternSyntax, SyntaxError> {
        Self::parse_range_with_binding_mode(tokens, bounds, true)
    }

    fn parse_range_with_binding_mode(
        tokens: &[Token],
        bounds: Range<usize>,
        bind_bare_identifiers: bool,
    ) -> Result<PatternSyntax, SyntaxError> {
        let mut cursor = TokenCursor::new(tokens, bounds)
            .ok_or_else(|| SyntaxError::new(SyntaxErrorKind::UnexpectedToken, 0..0))?;
        let pattern = Self::parse(&mut cursor, bind_bare_identifiers)?;
        if !cursor.is_empty() {
            return Err(PathParser::error_at(
                &cursor,
                SyntaxErrorKind::UnexpectedToken,
            ));
        }
        Ok(pattern)
    }

    fn parse(
        cursor: &mut TokenCursor<'_>,
        bind_bare_identifiers: bool,
    ) -> Result<PatternSyntax, SyntaxError> {
        let attributes = Self::parse_attributes(cursor)?;
        let pattern_start = cursor.index();
        let pattern_start_offset = cursor.offset();
        if cursor.consume_punctuation("|") {
            let first = Self::parse(cursor, bind_bare_identifiers)?;
            let alternatives = match first.kind() {
                PatternSyntaxKind::Or(alternatives) => alternatives.clone(),
                _ => vec![first.clone()],
            };
            let bounds = PathParser::span(
                cursor.tokens(),
                pattern_start,
                cursor.consumed_end_index(),
                pattern_start_offset,
                cursor.offset(),
            );
            let bounds = Self::include_attributes(bounds, &attributes);
            return Ok(PatternSyntax::new(
                PatternSyntaxKind::Or(alternatives),
                if attributes.is_empty() {
                    first.attributes().to_vec()
                } else {
                    attributes
                },
                bounds,
            ));
        }
        let kind = Self::parse_atom(cursor, bind_bare_identifiers)?;
        let bounds = Self::include_attributes(
            PathParser::span(
                cursor.tokens(),
                pattern_start,
                cursor.consumed_end_index(),
                pattern_start_offset,
                cursor.offset(),
            ),
            &attributes,
        );
        let mut pattern = PatternSyntax::new(kind, attributes.clone(), bounds);
        let inclusive = if cursor.consume_punctuation("..=") {
            Some(true)
        } else if cursor.consume_punctuation("..") {
            Some(false)
        } else {
            None
        };
        if let Some(inclusive) = inclusive {
            let end = if Self::is_pattern_boundary(cursor) {
                None
            } else {
                Some(Box::new(Self::parse(cursor, bind_bare_identifiers)?))
            };
            let bounds = PathParser::span(
                cursor.tokens(),
                pattern_start,
                cursor.consumed_end_index(),
                pattern_start_offset,
                cursor.offset(),
            );
            let bounds = Self::include_attributes(bounds, &attributes);
            pattern = PatternSyntax::new(
                PatternSyntaxKind::Range {
                    start: Some(Box::new(pattern)),
                    end,
                    inclusive,
                },
                Vec::new(),
                bounds,
            );
        }
        if cursor.consume_punctuation("|") {
            let mut alternatives = vec![pattern];
            alternatives.push(Self::parse(cursor, bind_bare_identifiers)?);
            while cursor.consume_punctuation("|") {
                alternatives.push(Self::parse(cursor, bind_bare_identifiers)?);
            }
            let first_start = alternatives
                .first()
                .map_or(pattern_start_offset, |value| value.bounds().start);
            let last_end = alternatives
                .last()
                .map_or(cursor.offset(), |value| value.bounds().end);
            return Ok(PatternSyntax::new(
                PatternSyntaxKind::Or(alternatives),
                attributes,
                first_start..last_end,
            ));
        }
        Ok(pattern)
    }

    fn parse_atom(
        cursor: &mut TokenCursor<'_>,
        bind_bare_identifiers: bool,
    ) -> Result<PatternSyntaxKind, SyntaxError> {
        if cursor.consume_punctuation("&") {
            let mutable = cursor.consume_identifier("mut");
            return Ok(PatternSyntaxKind::Reference {
                mutable,
                pattern: Box::new(Self::parse(cursor, bind_bare_identifiers)?),
            });
        }
        if cursor.at_identifier("ref") || cursor.at_identifier("mut") {
            let by_ref = cursor.consume_identifier("ref");
            let mutable = cursor.consume_identifier("mut");
            let token = cursor
                .peek()
                .ok_or_else(|| PathParser::error_at(cursor, SyntaxErrorKind::UnexpectedEof))?;
            let Some(name) = token.kind().identifier() else {
                return Err(PathParser::error_at(
                    cursor,
                    SyntaxErrorKind::UnexpectedToken,
                ));
            };
            let name = name.to_owned();
            cursor.consume();
            let subpattern = if cursor.consume_punctuation("@") {
                Some(Box::new(Self::parse(cursor, bind_bare_identifiers)?))
            } else {
                None
            };
            return Ok(PatternSyntaxKind::Binding {
                name,
                by_ref,
                mutable,
                subpattern,
            });
        }
        if cursor.consume_open_delimiter(Delimiter::Parenthesis) {
            let elements = Self::parse_pattern_list(cursor, ")", bind_bare_identifiers)?;
            return Ok(PatternSyntaxKind::Tuple(elements));
        }
        if cursor.consume_open_delimiter(Delimiter::Bracket) {
            let elements = Self::parse_pattern_list(cursor, "]", bind_bare_identifiers)?;
            return Ok(PatternSyntaxKind::Slice(elements));
        }
        if cursor.at_punctuation("..=") || cursor.at_punctuation("..") {
            let inclusive = cursor.consume_punctuation("..=");
            if !inclusive {
                cursor.consume_punctuation("..");
            }
            let end = if Self::is_pattern_boundary(cursor) {
                None
            } else {
                Some(Box::new(Self::parse(cursor, bind_bare_identifiers)?))
            };
            if end.is_none() && !inclusive {
                return Ok(PatternSyntaxKind::Rest);
            }
            return Ok(PatternSyntaxKind::Range {
                start: None,
                end,
                inclusive,
            });
        }
        if cursor.at_punctuation("-")
            || matches!(cursor.peek().map(Token::kind), Some(TokenKind::Literal(_)))
        {
            let literal_start = cursor.offset();
            if cursor.at_punctuation("-") {
                cursor.consume_punctuation("-");
            }
            let token = cursor
                .consume()
                .ok_or_else(|| PathParser::error_at(cursor, SyntaxErrorKind::UnexpectedEof))?;
            if !matches!(token.kind(), TokenKind::Literal(_)) {
                return Err(PathParser::error_at(
                    cursor,
                    SyntaxErrorKind::UnexpectedToken,
                ));
            }
            return Ok(PatternSyntaxKind::Literal(literal_start..token.end()));
        }
        if cursor.at_identifier("true") || cursor.at_identifier("false") {
            let token = cursor.consume().expect("布尔字面量 token 存在");
            return Ok(PatternSyntaxKind::Literal(token.start()..token.end()));
        }
        let path = PathParser::parse(cursor)?;
        if cursor.consume_punctuation("!") {
            let (delimiter, group) = Self::consume_group(cursor)?;
            return Ok(PatternSyntaxKind::Macro {
                path,
                delimiter,
                input: PathParser::span(
                    cursor.tokens(),
                    group.start,
                    group.end,
                    cursor.tokens()[group.start].start(),
                    cursor.tokens()[group.end - 1].end(),
                ),
            });
        }
        if cursor.consume_open_delimiter(Delimiter::Parenthesis) {
            return Ok(PatternSyntaxKind::TupleStruct {
                path,
                elements: Self::parse_pattern_list(cursor, ")", bind_bare_identifiers)?,
            });
        }
        if cursor.consume_open_delimiter(Delimiter::Brace) {
            return Self::parse_struct_fields(cursor, path, bind_bare_identifiers);
        }
        let simple_name = path
            .segments()
            .first()
            .filter(|_| path.segments().len() == 1 && !path.has_leading_colon())
            .map(|segment| segment.name().to_owned());
        if let Some(name) = simple_name.filter(|name| {
            name == "_"
                || bind_bare_identifiers
                || name.chars().next().is_some_and(char::is_lowercase)
        }) {
            if name == "_" {
                return Ok(PatternSyntaxKind::Wildcard);
            }
            let subpattern = if cursor.consume_punctuation("@") {
                Some(Box::new(Self::parse(cursor, bind_bare_identifiers)?))
            } else {
                None
            };
            return Ok(PatternSyntaxKind::Binding {
                name,
                by_ref: false,
                mutable: false,
                subpattern,
            });
        }
        if path.segments().len() == 1
            && path.segments()[0].name() == "_"
            && !path.has_leading_colon()
        {
            return Ok(PatternSyntaxKind::Wildcard);
        }
        Ok(PatternSyntaxKind::Path(path))
    }

    fn parse_pattern_list(
        cursor: &mut TokenCursor<'_>,
        closing: &str,
        bind_bare_identifiers: bool,
    ) -> Result<Vec<PatternSyntax>, SyntaxError> {
        let delimiter = match closing {
            ")" => Delimiter::Parenthesis,
            "]" => Delimiter::Bracket,
            "}" => Delimiter::Brace,
            _ => {
                return Err(PathParser::error_at(
                    cursor,
                    SyntaxErrorKind::UnexpectedToken,
                ))
            }
        };
        let mut patterns = Vec::new();
        while !cursor.is_empty() && !cursor.at_close_delimiter(delimiter) {
            patterns.push(Self::parse(cursor, bind_bare_identifiers)?);
            if !cursor.consume_punctuation(",") {
                break;
            }
        }
        if !cursor.consume_close_delimiter(delimiter) {
            return Err(PathParser::error_at(
                cursor,
                SyntaxErrorKind::UnexpectedToken,
            ));
        }
        Ok(patterns)
    }

    fn parse_struct_fields(
        cursor: &mut TokenCursor<'_>,
        path: PathSyntax,
        bind_bare_identifiers: bool,
    ) -> Result<PatternSyntaxKind, SyntaxError> {
        let mut fields = Vec::new();
        let mut rest = false;
        while !cursor.is_empty() && !cursor.at_close_delimiter(Delimiter::Brace) {
            let start = cursor.index();
            let start_offset = cursor.offset();
            let attributes = Self::parse_attributes(cursor)?;
            if cursor.consume_punctuation("..") {
                rest = true;
                if cursor.consume_punctuation(",") {
                    continue;
                }
                break;
            }
            let name = match cursor.consume() {
                Some(token) => match token.kind().identifier() {
                    Some(value) => value.to_owned(),
                    None => {
                        return Err(PathParser::error_at(
                            cursor,
                            SyntaxErrorKind::UnexpectedToken,
                        ))
                    }
                },
                None => return Err(PathParser::error_at(cursor, SyntaxErrorKind::UnexpectedEof)),
            };
            let mut field_name = name.clone();
            let mut by_ref = false;
            let mut mutable = false;
            if name == "ref" || name == "mut" {
                by_ref = name == "ref";
                mutable = name == "mut" || (by_ref && cursor.consume_identifier("mut"));
                let token = cursor
                    .consume()
                    .ok_or_else(|| PathParser::error_at(cursor, SyntaxErrorKind::UnexpectedEof))?;
                let Some(parsed_field_name) = token.kind().identifier() else {
                    return Err(PathParser::error_at(
                        cursor,
                        SyntaxErrorKind::UnexpectedToken,
                    ));
                };
                field_name = parsed_field_name.to_owned();
            }
            let pattern = if cursor.consume_punctuation(":") {
                Some(Self::parse(cursor, bind_bare_identifiers)?)
            } else {
                Some(PatternSyntax::new(
                    PatternSyntaxKind::Binding {
                        name: field_name.clone(),
                        by_ref,
                        mutable,
                        subpattern: None,
                    },
                    Vec::new(),
                    start_offset..cursor.offset(),
                ))
            };
            fields.push(PatternFieldSyntax::new(
                field_name,
                pattern,
                attributes,
                PathParser::span(
                    cursor.tokens(),
                    start,
                    cursor.consumed_end_index(),
                    start_offset,
                    cursor.offset(),
                ),
            ));
            if !cursor.consume_punctuation(",") {
                break;
            }
        }
        if !cursor.consume_close_delimiter(Delimiter::Brace) {
            return Err(PathParser::error_at(
                cursor,
                SyntaxErrorKind::UnexpectedToken,
            ));
        }
        Ok(PatternSyntaxKind::Struct { path, fields, rest })
    }

    fn parse_attributes(cursor: &mut TokenCursor<'_>) -> Result<Vec<Range<usize>>, SyntaxError> {
        let mut attributes = Vec::new();
        while cursor.at_punctuation("#") {
            let start = cursor.index();
            let start_offset = cursor.offset();
            cursor.consume_punctuation("#");
            cursor.consume_punctuation("!");
            if !cursor.at_open_delimiter(Delimiter::Bracket) {
                return Err(PathParser::error_at(
                    cursor,
                    SyntaxErrorKind::UnexpectedToken,
                ));
            }
            cursor
                .skip_delimited_group()
                .ok_or_else(|| PathParser::error_at(cursor, SyntaxErrorKind::UnclosedDelimiter))?;
            attributes.push(PathParser::span(
                cursor.tokens(),
                start,
                cursor.index(),
                start_offset,
                cursor.offset(),
            ));
        }
        Ok(attributes)
    }

    fn include_attributes(mut bounds: Range<usize>, attributes: &[Range<usize>]) -> Range<usize> {
        if let Some(start) = attributes.iter().map(|attribute| attribute.start).min() {
            bounds.start = bounds.start.min(start);
        }
        bounds
    }

    fn is_pattern_boundary(cursor: &TokenCursor<'_>) -> bool {
        matches!(
            cursor.peek().map(Token::kind),
            Some(TokenKind::CloseDelimiter(_))
        ) || cursor.at_punctuation(",")
            || cursor.at_punctuation("|")
    }

    fn consume_group(
        cursor: &mut TokenCursor<'_>,
    ) -> Result<(Delimiter, Range<usize>), SyntaxError> {
        let delimiter = match cursor.peek().map(Token::kind) {
            Some(TokenKind::OpenDelimiter(delimiter)) => *delimiter,
            _ => {
                return Err(PathParser::error_at(
                    cursor,
                    SyntaxErrorKind::UnexpectedToken,
                ))
            }
        };
        let group = cursor
            .skip_delimited_group()
            .ok_or_else(|| PathParser::error_at(cursor, SyntaxErrorKind::UnclosedDelimiter))?;
        Ok((delimiter, group))
    }
}
