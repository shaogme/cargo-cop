use std::ops::Range;

use crate::{
    ast::{
        ExpressionBlockKind, ExpressionSyntax, ExpressionSyntaxKind, ExpressionUnsupportedKind,
        UnaryOperator,
    },
    parser::{
        path::PathParser,
        token_cursor::{SyntaxError, SyntaxErrorKind, TokenCursor},
        types::TypeParser,
    },
    Delimiter, SourceFile, Token, TokenKind,
};

use super::{
    atoms::AtomParser,
    closure::{ClosureParser, ClosurePrefix},
    control::ControlParser,
};

struct OperatorBinding {
    left_power: u8,
    right_power: u8,
    assignment: bool,
    range: bool,
}

impl OperatorBinding {
    fn new(left_power: u8, right_power: u8, assignment: bool, range: bool) -> Self {
        Self {
            left_power,
            right_power,
            assignment,
            range,
        }
    }
}

pub(super) struct PrattParser;

impl PrattParser {
    pub(super) fn parse_complete(
        source: &SourceFile,
        tokens: &[Token],
        bounds: Range<usize>,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        let mut cursor = TokenCursor::new(tokens, bounds)
            .ok_or_else(|| SyntaxError::new(SyntaxErrorKind::UnexpectedToken, 0..0))?;
        let expression = Self::parse(source, &mut cursor, 0)?;
        if !cursor.is_empty() {
            return Err(PathParser::error_at(
                &cursor,
                SyntaxErrorKind::UnexpectedToken,
            ));
        }
        Ok(expression)
    }

    pub(super) fn parse_prefix(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        Self::parse(source, cursor, 0)
    }

    pub(super) fn parse(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
        min_binding_power: u8,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        Self::parse_with_struct_literals(source, cursor, min_binding_power, true)
    }

    pub(super) fn parse_control_header(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        Self::parse_with_struct_literals(source, cursor, 0, false)
    }

    pub(super) fn parse_with_struct_literals(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
        min_binding_power: u8,
        allow_struct_literals: bool,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        let attributes = Self::parse_expression_attributes(cursor)?;
        let mut left = Self::parse_prefix_expression(source, cursor, allow_struct_literals)?;
        if !attributes.is_empty() {
            left = left.with_outer_attributes(attributes);
        }
        loop {
            if let Some(postfix) = Self::parse_postfix(source, cursor, &left)? {
                left = postfix;
                continue;
            }
            if cursor.at_identifier("as") {
                let (left_bp, right_bp) = (13, 14);
                if left_bp < min_binding_power {
                    break;
                }
                cursor.consume_identifier("as");
                let target = TypeParser::parse(cursor)?;
                left = ExpressionSyntax::new(
                    ExpressionSyntaxKind::Cast {
                        expression: Box::new(left.clone()),
                        target,
                    },
                    left.bounds().start..cursor.consumed_offset(),
                );
                if right_bp < min_binding_power {
                    break;
                }
                continue;
            }
            let Some(operator) = Self::operator(cursor) else {
                break;
            };
            let Some(binding) = Self::binding_power(&operator) else {
                break;
            };
            if binding.left_power < min_binding_power {
                break;
            }
            cursor.consume();
            let right = if binding.range && Self::is_expression_boundary(cursor) {
                None
            } else {
                Some(Box::new(Self::parse_with_struct_literals(
                    source,
                    cursor,
                    binding.right_power,
                    allow_struct_literals,
                )?))
            };
            let bounds = left.bounds().start
                ..right
                    .as_ref()
                    .map_or_else(|| cursor.consumed_offset(), |value| value.bounds().end);
            left = if binding.range {
                ExpressionSyntax::new(
                    ExpressionSyntaxKind::Range {
                        start: Some(Box::new(left)),
                        end: right,
                        inclusive: operator == "..=",
                    },
                    bounds,
                )
            } else {
                let right = right.expect("二元运算符必须有右操作数");
                let kind = if binding.assignment {
                    ExpressionSyntaxKind::Assignment {
                        operator,
                        left: Box::new(left),
                        right,
                    }
                } else {
                    ExpressionSyntaxKind::Binary {
                        operator,
                        left: Box::new(left),
                        right,
                    }
                };
                ExpressionSyntax::new(kind, bounds)
            };
        }
        Ok(left)
    }

    pub(super) fn parse_expression_attributes(
        cursor: &mut TokenCursor<'_>,
    ) -> Result<Vec<Range<usize>>, SyntaxError> {
        Self::parse_attribute_ranges(cursor, false)
    }

    pub(super) fn parse_inner_attributes(
        cursor: &mut TokenCursor<'_>,
    ) -> Result<Vec<Range<usize>>, SyntaxError> {
        Self::parse_attribute_ranges(cursor, true)
    }

    fn parse_attribute_ranges(
        cursor: &mut TokenCursor<'_>,
        allow_inner: bool,
    ) -> Result<Vec<Range<usize>>, SyntaxError> {
        let mut attributes = Vec::new();
        while cursor.at_punctuation("#") {
            let checkpoint = cursor.checkpoint();
            let start = cursor.offset();
            cursor.consume_punctuation("#");
            let inner = cursor.consume_punctuation("!");
            if allow_inner && !inner {
                cursor.restore(checkpoint);
                break;
            }
            if inner && !allow_inner {
                return Err(PathParser::error_at(
                    cursor,
                    SyntaxErrorKind::UnexpectedToken,
                ));
            }
            if !cursor.at_open_delimiter(Delimiter::Bracket) {
                return Err(PathParser::error_at(
                    cursor,
                    SyntaxErrorKind::UnexpectedToken,
                ));
            }
            let (_, group) = Self::take_group(cursor)?;
            let end = cursor.tokens().get(group.end - 1).map_or(start, Token::end);
            attributes.push(start..end);
        }
        Ok(attributes)
    }

    fn parse_prefix_expression(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
        allow_struct_literals: bool,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        let start = cursor.index();
        let start_offset = cursor.offset();
        let token = cursor
            .peek()
            .ok_or_else(|| PathParser::error_at(cursor, SyntaxErrorKind::UnexpectedEof))?;
        match token.kind() {
            TokenKind::Literal(_) => {
                cursor.consume();
                Ok(Self::new_span(
                    cursor,
                    start,
                    start_offset,
                    ExpressionSyntaxKind::Literal,
                ))
            }
            TokenKind::Identifier(name) | TokenKind::WeakKeyword(name) => {
                let name = name.clone();
                match name.as_str() {
                    "if" => ControlParser::parse_if(source, cursor, start, start_offset),
                    "match" => ControlParser::parse_match(source, cursor, start, start_offset),
                    "loop" => ControlParser::parse_loop(cursor, start, start_offset, None),
                    "while" => {
                        ControlParser::parse_while(source, cursor, start, start_offset, None)
                    }
                    "for" => ControlParser::parse_for(source, cursor, start, start_offset, None),
                    "return" => ControlParser::parse_return(source, cursor, start, start_offset),
                    "break" => ControlParser::parse_break(source, cursor, start, start_offset),
                    "continue" => ControlParser::parse_continue(cursor, start, start_offset),
                    "let" => ControlParser::parse_let(
                        source,
                        cursor,
                        start,
                        start_offset,
                        allow_struct_literals,
                    ),
                    "unsafe" if Self::next_is_open(cursor, Delimiter::Brace) => {
                        cursor.consume_identifier("unsafe");
                        ControlParser::parse_block(
                            cursor,
                            start,
                            start_offset,
                            ExpressionBlockKind::Unsafe,
                        )
                    }
                    "const" if Self::next_is_open(cursor, Delimiter::Brace) => {
                        cursor.consume_identifier("const");
                        ControlParser::parse_block(
                            cursor,
                            start,
                            start_offset,
                            ExpressionBlockKind::Const,
                        )
                    }
                    "true" | "false" => {
                        cursor.consume();
                        Ok(Self::new_span(
                            cursor,
                            start,
                            start_offset,
                            ExpressionSyntaxKind::Literal,
                        ))
                    }
                    "_" => {
                        cursor.consume();
                        Ok(Self::new_span(
                            cursor,
                            start,
                            start_offset,
                            ExpressionSyntaxKind::Underscore,
                        ))
                    }
                    "move" if ClosureParser::is_closure_start(cursor) => {
                        cursor.consume();
                        ClosureParser::parse(
                            source,
                            cursor,
                            ClosurePrefix {
                                start,
                                start_offset,
                                is_async: false,
                                is_move: true,
                            },
                        )
                    }
                    _ => Self::parse_path_expression(
                        source,
                        cursor,
                        start,
                        start_offset,
                        allow_struct_literals,
                    ),
                }
            }
            TokenKind::EditionKeyword(name) if name == "async" => {
                ClosureParser::parse_async(source, cursor, start, start_offset)
            }
            TokenKind::ReservedKeyword(name)
                if name == "try" && Self::next_is_open(cursor, Delimiter::Brace) =>
            {
                cursor.consume_identifier("try");
                ControlParser::parse_unsupported_block(
                    cursor,
                    start,
                    start_offset,
                    ExpressionUnsupportedKind::TryBlock,
                )
            }
            TokenKind::EditionKeyword(_) | TokenKind::ReservedKeyword(_) => Err(
                PathParser::error_at(cursor, SyntaxErrorKind::UnexpectedToken),
            ),
            TokenKind::Lifetime(_)
                if cursor.peek_n(1).is_some_and(
                    |next| matches!(next.kind(), TokenKind::Punctuation(value) if value == ":"),
                ) =>
            {
                ControlParser::parse_labelled_block(source, cursor, start, start_offset)
            }
            TokenKind::Punctuation(operator) => match operator.as_str() {
                "(" => AtomParser::parse_parenthesized(source, cursor, start, start_offset),
                "[" => AtomParser::parse_array(source, cursor, start, start_offset),
                "{" => ControlParser::parse_block(
                    cursor,
                    start,
                    start_offset,
                    ExpressionBlockKind::Plain,
                ),
                "|" | "||" => ClosureParser::parse(
                    source,
                    cursor,
                    ClosurePrefix {
                        start,
                        start_offset,
                        is_async: false,
                        is_move: false,
                    },
                ),
                "-" | "!" | "*" | "&" => {
                    let operator = operator.clone();
                    let raw_borrow_operator = if operator == "&" {
                        Self::raw_borrow_operator(cursor)
                    } else {
                        None
                    };
                    cursor.consume();
                    let operator = match operator.as_str() {
                        "-" => UnaryOperator::Negate,
                        "!" => UnaryOperator::Not,
                        "*" => UnaryOperator::Dereference,
                        "&" => match raw_borrow_operator {
                            Some(operator) => {
                                cursor.consume_identifier("raw");
                                match operator {
                                    UnaryOperator::RawConstBorrow => {
                                        cursor.consume_identifier("const");
                                    }
                                    UnaryOperator::RawMutableBorrow => {
                                        cursor.consume_identifier("mut");
                                    }
                                    _ => unreachable!("原始借用前缀必须跟随 const 或 mut"),
                                }
                                operator
                            }
                            None if cursor.consume_identifier("mut") => {
                                UnaryOperator::MutableBorrow
                            }
                            None => UnaryOperator::SharedBorrow,
                        },
                        _ => unreachable!("一元运算符分支只接受已知标点"),
                    };
                    let operand = Self::parse_with_struct_literals(
                        source,
                        cursor,
                        14,
                        allow_struct_literals,
                    )?;
                    Ok(ExpressionSyntax::new(
                        ExpressionSyntaxKind::Unary {
                            operator,
                            operand: Box::new(operand.clone()),
                        },
                        start_offset..operand.bounds().end,
                    ))
                }
                ".." | "..=" => Self::parse_prefix_range(
                    source,
                    cursor,
                    start,
                    start_offset,
                    allow_struct_literals,
                ),
                "::" | "<" => Self::parse_path_expression(
                    source,
                    cursor,
                    start,
                    start_offset,
                    allow_struct_literals,
                ),
                _ => Err(PathParser::error_at(
                    cursor,
                    SyntaxErrorKind::UnexpectedToken,
                )),
            },
            TokenKind::OpenDelimiter(Delimiter::Parenthesis) => {
                AtomParser::parse_parenthesized(source, cursor, start, start_offset)
            }
            TokenKind::OpenDelimiter(Delimiter::Bracket) => {
                AtomParser::parse_array(source, cursor, start, start_offset)
            }
            TokenKind::OpenDelimiter(Delimiter::Brace) => {
                ControlParser::parse_block(cursor, start, start_offset, ExpressionBlockKind::Plain)
            }
            TokenKind::Unknown(_) | TokenKind::CloseDelimiter(_) | TokenKind::Lifetime(_) => Err(
                PathParser::error_at(cursor, SyntaxErrorKind::UnexpectedToken),
            ),
        }
    }

    fn parse_path_expression(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
        start: usize,
        start_offset: usize,
        allow_struct_literals: bool,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        let path = PathParser::parse_expression(cursor)?;
        if cursor.consume_punctuation("!") {
            let (delimiter, group) = Self::take_group(cursor)?;
            let input = Self::byte_span(cursor.tokens(), group.clone());
            let bounds = start_offset..cursor.consumed_offset();
            return Ok(ExpressionSyntax::new(
                ExpressionSyntaxKind::Macro {
                    path,
                    delimiter,
                    input,
                },
                bounds,
            ));
        }
        if allow_struct_literals && cursor.at_open_delimiter(Delimiter::Brace) {
            return AtomParser::parse_struct(source, cursor, start, start_offset, path);
        }
        let _ = start;
        let _ = source;
        Ok(ExpressionSyntax::new(
            ExpressionSyntaxKind::Path(path.clone()),
            path.bounds(),
        ))
    }

    fn parse_postfix(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
        left: &ExpressionSyntax,
    ) -> Result<Option<ExpressionSyntax>, SyntaxError> {
        let start = left.bounds().start;
        if cursor.consume_punctuation("?") {
            return Ok(Some(ExpressionSyntax::new(
                ExpressionSyntaxKind::Try(Box::new(left.clone())),
                start..cursor.consumed_offset(),
            )));
        }
        if cursor.consume_punctuation(".") {
            let member = cursor
                .peek()
                .ok_or_else(|| PathParser::error_at(cursor, SyntaxErrorKind::UnexpectedEof))?;
            if matches!(member.kind(), TokenKind::EditionKeyword(name) if name == "await") {
                cursor.consume();
                return Ok(Some(ExpressionSyntax::new(
                    ExpressionSyntaxKind::Await(Box::new(left.clone())),
                    start..cursor.consumed_offset(),
                )));
            }
            if member.kind().identifier().is_some() {
                let method_path = PathParser::parse_expression(cursor)?;
                let method_segment = method_path.segments().last().ok_or_else(|| {
                    PathParser::error_at(cursor, SyntaxErrorKind::UnexpectedToken)
                })?;
                if cursor.at_open_delimiter(Delimiter::Parenthesis) {
                    let arguments = Self::parse_arguments(source, cursor)?;
                    return Ok(Some(ExpressionSyntax::new(
                        ExpressionSyntaxKind::MethodCall {
                            receiver: Box::new(left.clone()),
                            method: method_segment.name().to_owned(),
                            generic_arguments: method_segment.arguments().to_vec(),
                            arguments,
                        },
                        start..cursor.consumed_offset(),
                    )));
                }
                return Ok(Some(ExpressionSyntax::new(
                    ExpressionSyntaxKind::Field {
                        base: Box::new(left.clone()),
                        field: method_segment.name().to_owned(),
                    },
                    start..cursor.consumed_offset(),
                )));
            }
            if let TokenKind::Literal(_) = member.kind() {
                let text = member.text(source);
                let Ok(index) = text.parse::<usize>() else {
                    return Err(PathParser::error_at(
                        cursor,
                        SyntaxErrorKind::UnexpectedToken,
                    ));
                };
                cursor.consume();
                return Ok(Some(ExpressionSyntax::new(
                    ExpressionSyntaxKind::TupleField {
                        base: Box::new(left.clone()),
                        index,
                    },
                    start..cursor.consumed_offset(),
                )));
            }
            return Err(PathParser::error_at(
                cursor,
                SyntaxErrorKind::UnexpectedToken,
            ));
        }
        if cursor.at_open_delimiter(Delimiter::Parenthesis) {
            let arguments = Self::parse_arguments(source, cursor)?;
            if let ExpressionSyntaxKind::Field { base, field } = left.kind() {
                return Ok(Some(ExpressionSyntax::new(
                    ExpressionSyntaxKind::MethodCall {
                        receiver: base.clone(),
                        method: field.clone(),
                        generic_arguments: Vec::new(),
                        arguments,
                    },
                    start..cursor.consumed_offset(),
                )));
            }
            return Ok(Some(ExpressionSyntax::new(
                ExpressionSyntaxKind::Call {
                    callee: Box::new(left.clone()),
                    arguments,
                },
                start..cursor.consumed_offset(),
            )));
        }
        if cursor.at_open_delimiter(Delimiter::Bracket) {
            let group = cursor
                .skip_delimited_group()
                .ok_or_else(|| PathParser::error_at(cursor, SyntaxErrorKind::UnclosedDelimiter))?;
            let index =
                Self::parse_complete(source, cursor.tokens(), group.start + 1..group.end - 1)?;
            return Ok(Some(ExpressionSyntax::new(
                ExpressionSyntaxKind::Index {
                    base: Box::new(left.clone()),
                    index: Box::new(index),
                },
                start..cursor.consumed_offset(),
            )));
        }
        Ok(None)
    }

    fn parse_arguments(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
    ) -> Result<Vec<ExpressionSyntax>, SyntaxError> {
        let (_, group) = Self::take_group(cursor)?;
        let mut arguments = Vec::new();
        let mut inner = TokenCursor::new(cursor.tokens(), group.start + 1..group.end - 1)
            .ok_or_else(|| PathParser::error_at(cursor, SyntaxErrorKind::UnexpectedToken))?;
        while !inner.is_empty() {
            arguments.push(Self::parse(source, &mut inner, 0)?);
            if !inner.consume_punctuation(",") && !inner.is_empty() {
                return Err(PathParser::error_at(
                    &inner,
                    SyntaxErrorKind::UnexpectedToken,
                ));
            }
        }
        Ok(arguments)
    }

    fn parse_prefix_range(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
        start: usize,
        start_offset: usize,
        allow_struct_literals: bool,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        let inclusive = cursor.consume_punctuation("..=");
        if !inclusive {
            cursor.consume_punctuation("..");
        }
        let end = if Self::is_expression_boundary(cursor) {
            None
        } else {
            Some(Box::new(Self::parse_with_struct_literals(
                source,
                cursor,
                3,
                allow_struct_literals,
            )?))
        };
        Ok(Self::new_span(
            cursor,
            start,
            start_offset,
            ExpressionSyntaxKind::Range {
                start: None,
                end,
                inclusive,
            },
        ))
    }

    pub(super) fn take_group(
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

    pub(super) fn find_top_level_token(
        tokens: &[Token],
        bounds: Range<usize>,
        expected: &str,
    ) -> Option<usize> {
        let mut index = bounds.start;
        while index < bounds.end {
            if matches!(tokens[index].kind(), TokenKind::OpenDelimiter(_)) {
                index = Self::matching_group(tokens, index, bounds.end)? + 1;
                continue;
            }
            if matches!(tokens[index].kind(), TokenKind::Punctuation(value) if value == expected)
                || matches!(tokens[index].kind(), TokenKind::Identifier(value) if value == expected)
            {
                return Some(index);
            }
            index += 1;
        }
        None
    }

    fn matching_group(tokens: &[Token], open: usize, end: usize) -> Option<usize> {
        let delimiter = match tokens.get(open)?.kind() {
            TokenKind::OpenDelimiter(delimiter) => *delimiter,
            _ => return None,
        };
        let mut depth = 0usize;
        for (index, token) in tokens.iter().enumerate().take(end).skip(open) {
            match token.kind() {
                TokenKind::OpenDelimiter(found) if *found == delimiter => depth += 1,
                TokenKind::CloseDelimiter(found) if *found == delimiter => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return Some(index);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn operator(cursor: &TokenCursor<'_>) -> Option<String> {
        match cursor.peek().map(Token::kind) {
            Some(TokenKind::Punctuation(operator)) if Self::binding_power(operator).is_some() => {
                Some(operator.clone())
            }
            _ => None,
        }
    }

    fn binding_power(operator: &str) -> Option<OperatorBinding> {
        let binding = match operator {
            "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "^=" | "&=" | "|=" | "<<=" | ">>=" => {
                OperatorBinding::new(1, 1, true, false)
            }
            ".." | "..=" => OperatorBinding::new(2, 3, false, true),
            "||" => OperatorBinding::new(3, 4, false, false),
            "&&" => OperatorBinding::new(4, 5, false, false),
            "|" => OperatorBinding::new(5, 6, false, false),
            "^" => OperatorBinding::new(6, 7, false, false),
            "&" => OperatorBinding::new(7, 8, false, false),
            "==" | "!=" | "<" | ">" | "<=" | ">=" => OperatorBinding::new(8, 9, false, false),
            "<<" | ">>" => OperatorBinding::new(9, 10, false, false),
            "+" | "-" => OperatorBinding::new(10, 11, false, false),
            "*" | "/" | "%" => OperatorBinding::new(11, 12, false, false),
            _ => return None,
        };
        Some(binding)
    }

    pub(super) fn is_expression_boundary(cursor: &TokenCursor<'_>) -> bool {
        cursor.is_empty()
            || cursor.at_punctuation(";")
            || cursor.at_punctuation(",")
            || matches!(
                cursor.peek().map(Token::kind),
                Some(TokenKind::CloseDelimiter(_))
            )
    }

    fn next_is_open(cursor: &TokenCursor<'_>, delimiter: Delimiter) -> bool {
        cursor.peek_n(1).is_some_and(
            |token| matches!(token.kind(), TokenKind::OpenDelimiter(found) if *found == delimiter),
        )
    }

    fn raw_borrow_operator(cursor: &TokenCursor<'_>) -> Option<UnaryOperator> {
        if cursor.peek_n(1)?.kind().word() != Some("raw") {
            return None;
        }
        match cursor.peek_n(2)?.kind().word()? {
            "const" => Some(UnaryOperator::RawConstBorrow),
            "mut" => Some(UnaryOperator::RawMutableBorrow),
            _ => None,
        }
    }

    pub(super) fn new_span(
        cursor: &TokenCursor<'_>,
        start: usize,
        start_offset: usize,
        kind: ExpressionSyntaxKind,
    ) -> ExpressionSyntax {
        let span = Self::span(cursor.tokens(), start, cursor.consumed_end_index());
        ExpressionSyntax::new(
            kind,
            span.start.min(start_offset)..span.end.max(start_offset),
        )
    }

    pub(super) fn span(tokens: &[Token], start: usize, end: usize) -> Range<usize> {
        let start = tokens.get(start).map_or(0, Token::start);
        let end = end
            .checked_sub(1)
            .and_then(|index| tokens.get(index))
            .map_or(start, Token::end);
        start..end.max(start)
    }

    pub(super) fn byte_span(tokens: &[Token], bounds: Range<usize>) -> Range<usize> {
        Self::span(tokens, bounds.start, bounds.end)
    }
}
