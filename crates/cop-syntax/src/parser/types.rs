use std::ops::Range;

use cop_model::{LocalizedMessage, SYNTAX_ITEM_UNSUPPORTED, SYNTAX_MACRO_OPAQUE};

use crate::{
    ast::{
        AstFactory, AstNodeId, FunctionTraitArgumentsSyntax, GenericArgumentSyntax,
        MacroInputStatus, ParseIssue, ParseIssueKind, PathSyntax, SyntaxNode, SyntaxNodeData,
        SyntaxNodeKind, TraitBoundKind, TraitBoundSyntax, TypeSyntax, TypeSyntaxKind,
    },
    Delimiter, Token, TokenKind,
};

use super::{
    path::PathParser,
    token_cursor::{SyntaxError, SyntaxErrorKind, TokenCursor},
};

pub(super) struct TypeInput<'a> {
    pub(super) tokens: &'a [Token],
    pub(super) bounds: Range<usize>,
    pub(super) parent: AstNodeId,
    pub(super) ast_nodes: &'a mut Vec<SyntaxNode>,
    pub(super) issues: &'a mut Vec<ParseIssue>,
}

pub(super) struct ParsedTypeInput<'a> {
    pub(super) syntax: TypeSyntax,
    pub(super) tokens: &'a [Token],
    pub(super) parent: AstNodeId,
    pub(super) ast_nodes: &'a mut Vec<SyntaxNode>,
    pub(super) issues: &'a mut Vec<ParseIssue>,
}

pub(super) struct TypeParser;

impl TypeParser {
    pub(super) fn collect_parsed(input: ParsedTypeInput<'_>) -> AstNodeId {
        let ParsedTypeInput {
            syntax,
            tokens,
            parent,
            ast_nodes,
            issues,
        } = input;
        let id = Self::push_node(syntax.clone(), parent, ast_nodes);
        Self::emit_opaque_macros(&syntax, id, tokens, ast_nodes, issues);
        id
    }

    pub(super) fn collect(input: TypeInput<'_>) -> Option<AstNodeId> {
        let TypeInput {
            tokens,
            bounds,
            parent,
            ast_nodes,
            issues,
        } = input;
        match Self::parse_range(tokens, bounds.clone()) {
            Ok(syntax) => {
                let id = Self::push_node(syntax.clone(), parent, ast_nodes);
                Self::emit_opaque_macros(&syntax, id, tokens, ast_nodes, issues);
                Some(id)
            }
            Err(error) => {
                Self::record_error(error, parent, ast_nodes, issues);
                None
            }
        }
    }

    pub(super) fn collect_prefix(input: TypeInput<'_>) -> Option<AstNodeId> {
        let TypeInput {
            tokens,
            bounds,
            parent,
            ast_nodes,
            issues,
        } = input;
        let mut cursor = TokenCursor::new(tokens, bounds)?;
        match Self::parse(&mut cursor) {
            Ok(syntax) => {
                let id = Self::push_node(syntax.clone(), parent, ast_nodes);
                Self::emit_opaque_macros(&syntax, id, tokens, ast_nodes, issues);
                Some(id)
            }
            Err(error) => {
                Self::record_error(error, parent, ast_nodes, issues);
                None
            }
        }
    }

    pub(super) fn collect_bounds(input: TypeInput<'_>) -> Option<AstNodeId> {
        let TypeInput {
            tokens,
            bounds,
            parent,
            ast_nodes,
            issues,
        } = input;
        let start_offset = tokens.get(bounds.start).map_or(0, Token::start);
        let end_offset = bounds
            .end
            .checked_sub(1)
            .and_then(|index| tokens.get(index))
            .map_or(start_offset, Token::end);
        let mut cursor = TokenCursor::new(tokens, bounds)
            .ok_or_else(|| SyntaxError::new(SyntaxErrorKind::UnexpectedToken, 0..0))
            .ok()?;
        match Self::parse_bounds(&mut cursor) {
            Ok(parsed) if cursor.is_empty() => {
                let syntax = TypeSyntax::new(
                    TypeSyntaxKind::TraitBounds(parsed),
                    start_offset..end_offset,
                );
                let id = Self::push_node(syntax.clone(), parent, ast_nodes);
                Self::emit_opaque_macros(&syntax, id, tokens, ast_nodes, issues);
                Some(id)
            }
            Ok(_) => {
                let error = PathParser::error_at(&cursor, SyntaxErrorKind::UnexpectedToken);
                Self::record_error(error, parent, ast_nodes, issues);
                None
            }
            Err(error) => {
                Self::record_error(error, parent, ast_nodes, issues);
                None
            }
        }
    }

    pub(super) fn parse(cursor: &mut TokenCursor<'_>) -> Result<TypeSyntax, SyntaxError> {
        let start = cursor.index();
        let start_offset = cursor.offset();
        let kind = if cursor.consume_punctuation("&") {
            Self::parse_reference(cursor)?
        } else if cursor.consume_punctuation("*") {
            Self::parse_pointer(cursor)?
        } else if cursor.consume_punctuation("!") {
            TypeSyntaxKind::Never
        } else if cursor.consume_identifier("_") {
            TypeSyntaxKind::Infer
        } else if cursor.consume_open_delimiter(Delimiter::Parenthesis) {
            Self::parse_parenthesized(cursor)?
        } else if cursor.consume_open_delimiter(Delimiter::Bracket) {
            Self::parse_bracketed(cursor)?
        } else if cursor.at_identifier("impl") {
            cursor.consume_identifier("impl");
            TypeSyntaxKind::ImplTrait(Self::parse_bounds(cursor)?)
        } else if Self::is_dyn_type_prefix(cursor) {
            cursor.consume_identifier("dyn");
            TypeSyntaxKind::DynTrait(Self::parse_bounds(cursor)?)
        } else if cursor.at_identifier("fn")
            || cursor.at_identifier("unsafe")
            || cursor.at_identifier("extern")
            || cursor.at_identifier("for")
        {
            Self::parse_function_pointer(cursor)?
        } else {
            Self::parse_path_or_macro(cursor)?
        };
        let bounds = PathParser::span(
            cursor.tokens(),
            start,
            cursor.consumed_end_index(),
            start_offset,
            cursor.offset(),
        );
        Ok(TypeSyntax::new(kind, bounds))
    }

    pub(super) fn parse_range(
        tokens: &[Token],
        bounds: Range<usize>,
    ) -> Result<TypeSyntax, SyntaxError> {
        let base = TokenCursor::new(tokens, bounds)
            .ok_or_else(|| SyntaxError::new(SyntaxErrorKind::UnexpectedToken, 0..0))?;
        let mut cursor = base
            .subcursor(base.start()..base.end())
            .ok_or_else(|| SyntaxError::new(SyntaxErrorKind::UnexpectedToken, 0..0))?;
        let value = match Self::parse(&mut cursor) {
            Ok(value) => value,
            Err(error) => {
                cursor.recover_to(&[",", ";", "=", "where"]);
                return Err(error);
            }
        };
        if !cursor.is_empty() {
            let error = PathParser::error_at(&cursor, SyntaxErrorKind::UnexpectedToken);
            cursor.recover_to(&[",", ";", "=", "where"]);
            return Err(error);
        }
        Ok(value)
    }

    pub(super) fn parse_bounds(
        cursor: &mut TokenCursor<'_>,
    ) -> Result<Vec<TraitBoundSyntax>, SyntaxError> {
        let mut bounds = Vec::new();
        loop {
            bounds.push(Self::parse_bound(cursor)?);
            if !cursor.consume_punctuation("+") {
                break;
            }
        }
        Ok(bounds)
    }

    fn is_dyn_type_prefix(cursor: &TokenCursor<'_>) -> bool {
        match cursor.peek().map(Token::kind) {
            Some(TokenKind::EditionKeyword(name)) if name == "dyn" => true,
            Some(TokenKind::WeakKeyword(name)) if name == "dyn" => {
                cursor.peek_n(1).is_some_and(|next| match next.kind() {
                    TokenKind::Lifetime(_) => true,
                    kind if kind.identifier().is_some() => true,
                    TokenKind::Punctuation(value) => value == "?",
                    TokenKind::OpenDelimiter(Delimiter::Parenthesis) => true,
                    _ => false,
                })
            }
            _ => false,
        }
    }

    fn parse_reference(cursor: &mut TokenCursor<'_>) -> Result<TypeSyntaxKind, SyntaxError> {
        let lifetime = match cursor.peek().map(Token::kind) {
            Some(TokenKind::Lifetime(name)) => {
                let name = name.clone();
                cursor.consume();
                Some(name)
            }
            _ => None,
        };
        let mutable = cursor.consume_identifier("mut");
        let target = Self::parse(cursor)?;
        Ok(TypeSyntaxKind::Reference {
            lifetime,
            mutable,
            target: Box::new(target),
        })
    }

    fn push_node(
        syntax: TypeSyntax,
        parent: AstNodeId,
        ast_nodes: &mut Vec<SyntaxNode>,
    ) -> AstNodeId {
        AstFactory::push_data_node(
            ast_nodes,
            parent,
            SyntaxNodeKind::Type,
            syntax.bounds(),
            SyntaxNodeData::Type(syntax),
        )
    }

    pub(super) fn emit_opaque_macros(
        syntax: &TypeSyntax,
        parent: AstNodeId,
        tokens: &[Token],
        ast_nodes: &mut Vec<SyntaxNode>,
        issues: &mut Vec<ParseIssue>,
    ) {
        match syntax.kind() {
            TypeSyntaxKind::Path(path) => {
                Self::emit_path_argument_macros(path, parent, tokens, ast_nodes, issues);
            }
            TypeSyntaxKind::Reference { target, .. }
            | TypeSyntaxKind::Pointer { target, .. }
            | TypeSyntaxKind::Parenthesized(target)
            | TypeSyntaxKind::Slice(target) => {
                Self::emit_opaque_macros(target, parent, tokens, ast_nodes, issues);
            }
            TypeSyntaxKind::Tuple(types) => {
                for ty in types {
                    Self::emit_opaque_macros(ty, parent, tokens, ast_nodes, issues);
                }
            }
            TypeSyntaxKind::Array { element, .. } => {
                Self::emit_opaque_macros(element, parent, tokens, ast_nodes, issues);
            }
            TypeSyntaxKind::FunctionPointer {
                parameters,
                return_type,
                ..
            } => {
                for parameter in parameters {
                    Self::emit_opaque_macros(parameter, parent, tokens, ast_nodes, issues);
                }
                if let Some(return_type) = return_type {
                    Self::emit_opaque_macros(return_type, parent, tokens, ast_nodes, issues);
                }
            }
            TypeSyntaxKind::ImplTrait(bounds)
            | TypeSyntaxKind::DynTrait(bounds)
            | TypeSyntaxKind::TraitBounds(bounds) => {
                for bound in bounds {
                    Self::emit_bound_argument_macros(bound, parent, tokens, ast_nodes, issues);
                }
            }
            TypeSyntaxKind::Macro { path, input, .. } => {
                let invocation = AstFactory::macro_invocation(
                    path.clone(),
                    syntax.bounds(),
                    tokens,
                    input.clone(),
                    MacroInputStatus::Opaque,
                );
                let macro_id = AstFactory::push_data_node(
                    ast_nodes,
                    parent,
                    SyntaxNodeKind::Macro,
                    syntax.bounds(),
                    SyntaxNodeData::Macro(invocation),
                );
                let input_id = AstFactory::push_ast_node(
                    ast_nodes,
                    Some(macro_id),
                    SyntaxNodeKind::MacroInput,
                    input.clone(),
                    Vec::new(),
                );
                AstFactory::push_ast_node(
                    ast_nodes,
                    Some(input_id),
                    SyntaxNodeKind::Opaque,
                    input.clone(),
                    Vec::new(),
                );
                issues.push(AstFactory::parse_issue(
                    ParseIssueKind::UnsupportedSyntax,
                    input.clone(),
                    LocalizedMessage::new(SYNTAX_MACRO_OPAQUE),
                ));
                Self::emit_path_argument_macros(path, parent, tokens, ast_nodes, issues);
            }
            TypeSyntaxKind::Never | TypeSyntaxKind::Infer => {}
        }
    }

    pub(super) fn emit_path_argument_macros(
        path: &PathSyntax,
        parent: AstNodeId,
        tokens: &[Token],
        ast_nodes: &mut Vec<SyntaxNode>,
        issues: &mut Vec<ParseIssue>,
    ) {
        if let Some(self_type) = path.qualified_self() {
            Self::emit_opaque_macros(self_type, parent, tokens, ast_nodes, issues);
        }
        if let Some(trait_path) = path.qualified_trait() {
            Self::emit_path_argument_macros(trait_path, parent, tokens, ast_nodes, issues);
        }
        for segment in path.segments() {
            for argument in segment.arguments() {
                match argument {
                    GenericArgumentSyntax::Type(ty) => {
                        Self::emit_opaque_macros(ty, parent, tokens, ast_nodes, issues);
                    }
                    GenericArgumentSyntax::AssociatedType { bounds, value, .. } => {
                        for bound in bounds {
                            Self::emit_bound_argument_macros(
                                bound, parent, tokens, ast_nodes, issues,
                            );
                        }
                        Self::emit_opaque_macros(value, parent, tokens, ast_nodes, issues);
                    }
                    GenericArgumentSyntax::Lifetime(_)
                    | GenericArgumentSyntax::Const(_)
                    | GenericArgumentSyntax::AssociatedConst { .. } => {}
                }
            }
        }
    }

    pub(super) fn emit_generic_argument_macros(
        arguments: &[GenericArgumentSyntax],
        parent: AstNodeId,
        tokens: &[Token],
        ast_nodes: &mut Vec<SyntaxNode>,
        issues: &mut Vec<ParseIssue>,
    ) {
        for argument in arguments {
            match argument {
                GenericArgumentSyntax::Type(ty) => {
                    Self::emit_opaque_macros(ty, parent, tokens, ast_nodes, issues);
                }
                GenericArgumentSyntax::AssociatedType { bounds, value, .. } => {
                    for bound in bounds {
                        Self::emit_bound_argument_macros(bound, parent, tokens, ast_nodes, issues);
                    }
                    Self::emit_opaque_macros(value, parent, tokens, ast_nodes, issues);
                }
                GenericArgumentSyntax::Lifetime(_)
                | GenericArgumentSyntax::Const(_)
                | GenericArgumentSyntax::AssociatedConst { .. } => {}
            }
        }
    }

    fn emit_bound_argument_macros(
        bound: &TraitBoundSyntax,
        parent: AstNodeId,
        tokens: &[Token],
        ast_nodes: &mut Vec<SyntaxNode>,
        issues: &mut Vec<ParseIssue>,
    ) {
        if let TraitBoundKind::Trait(path) = bound.kind() {
            Self::emit_path_argument_macros(path, parent, tokens, ast_nodes, issues);
        }
        if let Some(arguments) = bound.function_arguments() {
            for parameter in arguments.parameters() {
                Self::emit_opaque_macros(parameter, parent, tokens, ast_nodes, issues);
            }
            if let Some(return_type) = arguments.return_type() {
                Self::emit_opaque_macros(return_type, parent, tokens, ast_nodes, issues);
            }
        }
    }

    fn record_error(
        error: SyntaxError,
        parent: AstNodeId,
        ast_nodes: &mut Vec<SyntaxNode>,
        issues: &mut Vec<ParseIssue>,
    ) {
        let error_bounds = error.bounds();
        let issue_kind = if error.kind() == SyntaxErrorKind::UnclosedDelimiter {
            ParseIssueKind::UnclosedDelimiter
        } else {
            ParseIssueKind::UnexpectedToken
        };
        if error_bounds.start < error_bounds.end {
            AstFactory::push_ast_node(
                ast_nodes,
                Some(parent),
                SyntaxNodeKind::Opaque,
                error_bounds.clone(),
                Vec::new(),
            );
        }
        issues.push(AstFactory::parse_issue(
            issue_kind,
            error_bounds,
            LocalizedMessage::new(SYNTAX_ITEM_UNSUPPORTED),
        ));
    }

    fn parse_pointer(cursor: &mut TokenCursor<'_>) -> Result<TypeSyntaxKind, SyntaxError> {
        let mutable = if cursor.consume_identifier("const") {
            false
        } else if cursor.consume_identifier("mut") {
            true
        } else {
            return Err(PathParser::error_at(
                cursor,
                SyntaxErrorKind::UnexpectedToken,
            ));
        };
        let target = Self::parse(cursor)?;
        Ok(TypeSyntaxKind::Pointer {
            mutable,
            target: Box::new(target),
        })
    }

    fn parse_parenthesized(cursor: &mut TokenCursor<'_>) -> Result<TypeSyntaxKind, SyntaxError> {
        let mut members = Vec::new();
        if cursor.consume_close_delimiter(Delimiter::Parenthesis) {
            return Ok(TypeSyntaxKind::Tuple(members));
        }
        members.push(Self::parse(cursor)?);
        let mut is_tuple = false;
        while cursor.consume_punctuation(",") {
            is_tuple = true;
            if cursor.at_close_delimiter(Delimiter::Parenthesis) {
                break;
            }
            members.push(Self::parse(cursor)?);
        }
        if !cursor.consume_close_delimiter(Delimiter::Parenthesis) {
            return Err(PathParser::error_at(
                cursor,
                SyntaxErrorKind::UnexpectedToken,
            ));
        }
        if is_tuple {
            Ok(TypeSyntaxKind::Tuple(members))
        } else {
            let inner = members
                .pop()
                .ok_or_else(|| PathParser::error_at(cursor, SyntaxErrorKind::UnexpectedToken))?;
            Ok(TypeSyntaxKind::Parenthesized(Box::new(inner)))
        }
    }

    fn parse_bracketed(cursor: &mut TokenCursor<'_>) -> Result<TypeSyntaxKind, SyntaxError> {
        let element = Self::parse(cursor)?;
        if cursor.consume_punctuation(";") {
            let length = Self::consume_const_until(cursor, "]")?;
            if !cursor.consume_close_delimiter(Delimiter::Bracket) {
                return Err(PathParser::error_at(
                    cursor,
                    SyntaxErrorKind::UnexpectedToken,
                ));
            }
            return Ok(TypeSyntaxKind::Array {
                element: Box::new(element),
                length,
            });
        }
        if !cursor.consume_close_delimiter(Delimiter::Bracket) {
            return Err(PathParser::error_at(
                cursor,
                SyntaxErrorKind::UnexpectedToken,
            ));
        }
        Ok(TypeSyntaxKind::Slice(Box::new(element)))
    }

    fn parse_function_pointer(cursor: &mut TokenCursor<'_>) -> Result<TypeSyntaxKind, SyntaxError> {
        let lifetimes = if cursor.consume_identifier("for") {
            Self::parse_lifetime_binders(cursor)?
        } else {
            Vec::new()
        };
        let unsafe_ = cursor.consume_identifier("unsafe");
        let abi = if cursor.consume_identifier("extern") {
            match cursor.peek().map(Token::kind) {
                Some(TokenKind::Literal(_)) => {
                    let literal = cursor.consume().expect("peeked token exists");
                    Some(literal.start()..literal.end())
                }
                _ => None,
            }
        } else {
            None
        };
        if !cursor.consume_identifier("fn")
            || !cursor.consume_open_delimiter(Delimiter::Parenthesis)
        {
            return Err(PathParser::error_at(
                cursor,
                SyntaxErrorKind::UnexpectedToken,
            ));
        }
        let mut parameters = Vec::new();
        let mut variadic = false;
        while !cursor.is_empty() && !cursor.at_close_delimiter(Delimiter::Parenthesis) {
            if cursor.consume_punctuation("...") {
                variadic = true;
                break;
            }
            let checkpoint = cursor.checkpoint();
            if cursor
                .peek()
                .is_some_and(|token| token.kind().identifier().is_some())
                && cursor.peek_n(1).is_some_and(
                    |token| matches!(token.kind(), TokenKind::Punctuation(value) if value == ":"),
                )
            {
                cursor.consume();
                cursor.consume_punctuation(":");
            } else {
                cursor.restore(checkpoint);
            }
            parameters.push(Self::parse(cursor)?);
            if !cursor.consume_punctuation(",") {
                break;
            }
        }
        if !cursor.consume_close_delimiter(Delimiter::Parenthesis) {
            return Err(PathParser::error_at(
                cursor,
                SyntaxErrorKind::UnexpectedToken,
            ));
        }
        let return_type = if cursor.consume_punctuation("->") {
            Some(Box::new(Self::parse(cursor)?))
        } else {
            None
        };
        Ok(TypeSyntaxKind::FunctionPointer {
            lifetimes,
            unsafe_,
            abi,
            parameters,
            variadic,
            return_type,
        })
    }

    fn parse_lifetime_binders(cursor: &mut TokenCursor<'_>) -> Result<Vec<String>, SyntaxError> {
        if !cursor.consume_angle_open() {
            return Err(PathParser::error_at(
                cursor,
                SyntaxErrorKind::UnexpectedToken,
            ));
        }
        let mut lifetimes = Vec::new();
        loop {
            match cursor.peek().map(Token::kind) {
                Some(TokenKind::Lifetime(name)) => {
                    lifetimes.push(name.clone());
                    cursor.consume();
                }
                _ => {
                    return Err(PathParser::error_at(
                        cursor,
                        SyntaxErrorKind::UnexpectedToken,
                    ))
                }
            }
            if !cursor.consume_punctuation(",") {
                break;
            }
        }
        if !cursor.consume_angle_close() {
            return Err(PathParser::error_at(
                cursor,
                SyntaxErrorKind::UnexpectedToken,
            ));
        }
        Ok(lifetimes)
    }

    fn parse_bound(cursor: &mut TokenCursor<'_>) -> Result<TraitBoundSyntax, SyntaxError> {
        let maybe = cursor.consume_punctuation("?");
        let lifetimes = if cursor.consume_identifier("for") {
            Self::parse_lifetime_binders(cursor)?
        } else {
            Vec::new()
        };
        let kind = match cursor.peek().map(Token::kind) {
            Some(TokenKind::Lifetime(name)) => {
                let name = name.clone();
                cursor.consume();
                let bound = TraitBoundKind::Lifetime(name);
                return Ok(TraitBoundSyntax::new(maybe, lifetimes, bound, None));
            }
            _ => TraitBoundKind::Trait(PathParser::parse(cursor)?),
        };
        let function_arguments = if cursor.consume_open_delimiter(Delimiter::Parenthesis) {
            let start = cursor.index().saturating_sub(1);
            let start_offset = cursor.tokens()[start].start();
            let mut parameters = Vec::new();
            while !cursor.is_empty() && !cursor.at_close_delimiter(Delimiter::Parenthesis) {
                parameters.push(Self::parse(cursor)?);
                if !cursor.consume_punctuation(",") {
                    break;
                }
            }
            if !cursor.consume_close_delimiter(Delimiter::Parenthesis) {
                return Err(PathParser::error_at(
                    cursor,
                    SyntaxErrorKind::UnexpectedToken,
                ));
            }
            let return_type = if cursor.consume_punctuation("->") {
                Some(Box::new(Self::parse(cursor)?))
            } else {
                None
            };
            let bounds = PathParser::span(
                cursor.tokens(),
                start,
                cursor.consumed_end_index(),
                start_offset,
                cursor.offset(),
            );
            Some(FunctionTraitArgumentsSyntax::new(
                parameters,
                return_type,
                bounds,
            ))
        } else {
            None
        };
        Ok(TraitBoundSyntax::new(
            maybe,
            lifetimes,
            kind,
            function_arguments,
        ))
    }

    fn parse_path_or_macro(cursor: &mut TokenCursor<'_>) -> Result<TypeSyntaxKind, SyntaxError> {
        let path = PathParser::parse(cursor)?;
        if !cursor.consume_punctuation("!") {
            return Ok(TypeSyntaxKind::Path(path));
        }
        let start = cursor.index();
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
        let input = PathParser::span(
            cursor.tokens(),
            start,
            group.end,
            cursor.tokens()[group.start].start(),
            cursor.tokens()[group.end - 1].end(),
        );
        Ok(TypeSyntaxKind::Macro {
            path,
            delimiter,
            input,
        })
    }

    fn consume_const_until(
        cursor: &mut TokenCursor<'_>,
        closing: &str,
    ) -> Result<Range<usize>, SyntaxError> {
        let start = cursor.index();
        let start_offset = cursor.offset();
        while !cursor.is_empty()
            && !(closing == "]" && cursor.at_close_delimiter(Delimiter::Bracket))
        {
            if matches!(
                cursor.peek().map(Token::kind),
                Some(TokenKind::OpenDelimiter(_))
            ) {
                if cursor.skip_delimited_group().is_none() {
                    return Err(PathParser::error_at(
                        cursor,
                        SyntaxErrorKind::UnclosedDelimiter,
                    ));
                }
            } else {
                cursor.consume();
            }
        }
        Ok(PathParser::span(
            cursor.tokens(),
            start,
            cursor.index(),
            start_offset,
            cursor.offset(),
        ))
    }
}
