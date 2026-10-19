use std::ops::Range;

use cop_model::{
    LocalizedMessage, SYNTAX_EXPRESSION_UNSUPPORTED, SYNTAX_ITEM_UNSUPPORTED,
    SYNTAX_PARSE_ERROR_UNEXPECTED_TOKEN,
};

use crate::{
    ast::{
        AstFactory, AstNodeId, Attribute, ExpressionErrorKind, ExpressionSyntax,
        ExpressionSyntaxKind, ParseIssue, ParseIssueCode, ParseIssueKind, PatternSyntax,
        PatternSyntaxKind, SyntaxNode, SyntaxNodeData, SyntaxNodeKind, UnsupportedConstructKind,
    },
    parser::{
        patterns::PatternParser,
        token_cursor::{SyntaxError, SyntaxErrorKind},
        types::TypeParser,
    },
    recovery::Recovery,
    RustEdition, SourceFile, Token, TokenKind,
};

use super::{
    edition::EditionParser, macro_support::MacroInputSupport, pratt::PrattParser, AttributeParser,
    ParameterPatternInput, PatternInput, SyntaxRequest,
};

pub(super) struct ExpressionTree;

struct ParseIssueDetails {
    kind: ParseIssueKind,
    code: ParseIssueCode,
    boundary_unknown: bool,
}

pub(super) struct EmitContext<'a> {
    pub(super) source: &'a SourceFile,
    pub(super) tokens: &'a [Token],
    pub(super) edition: Option<RustEdition>,
    pub(super) ast_nodes: &'a mut Vec<SyntaxNode>,
    pub(super) issues: &'a mut Vec<ParseIssue>,
}

impl ExpressionTree {
    pub(super) fn collect_expression(
        request: SyntaxRequest<'_>,
        ast_nodes: &mut Vec<SyntaxNode>,
        issues: &mut Vec<ParseIssue>,
    ) -> Option<AstNodeId> {
        let SyntaxRequest {
            source,
            tokens,
            bounds,
            parent,
            edition,
            ..
        } = request;
        let first = tokens.get(bounds.start)?;
        let last = bounds
            .end
            .checked_sub(1)
            .and_then(|index| tokens.get(index))?;
        let mut expression_start = bounds.start;
        let attributes =
            AttributeParser::parse_attributes(source, tokens, &mut expression_start, issues);
        let expression_node = AstFactory::push_ast_node(
            ast_nodes,
            Some(parent),
            SyntaxNodeKind::Expression,
            first.start()..last.end(),
            attributes.clone(),
        );
        if expression_start >= bounds.end {
            Self::push_opaque_issue(
                ast_nodes,
                expression_node,
                first.start()..last.end(),
                issues,
            );
            return Some(expression_node);
        }
        match PrattParser::parse_complete(source, tokens, expression_start..bounds.end) {
            Ok(expression) => {
                let attribute_ranges = attributes
                    .iter()
                    .map(|attribute| attribute.start()..attribute.end())
                    .collect();
                let expression = expression.with_outer_attributes(attribute_ranges);
                AstFactory::set_ast_node_data(
                    ast_nodes,
                    expression_node,
                    SyntaxNodeData::Expression(expression.clone()),
                );
                let mut context = EmitContext {
                    source,
                    tokens,
                    edition,
                    ast_nodes,
                    issues,
                };
                Self::emit_kind_children(&expression, expression_node, &mut context);
            }
            Err(error) => {
                let kind = if error.kind() == SyntaxErrorKind::UnclosedDelimiter {
                    ParseIssueKind::UnclosedDelimiter
                } else {
                    ParseIssueKind::UnexpectedToken
                };
                if error.bounds().start < error.bounds().end {
                    AstFactory::push_ast_node(
                        ast_nodes,
                        Some(expression_node),
                        SyntaxNodeKind::Opaque,
                        error.bounds().clone(),
                        Vec::new(),
                    );
                }
                issues.push(AstFactory::parse_issue_with_boundary(
                    kind,
                    error.bounds(),
                    LocalizedMessage::new(SYNTAX_ITEM_UNSUPPORTED),
                    error.kind() == SyntaxErrorKind::UnclosedDelimiter,
                ));
            }
        }
        Some(expression_node)
    }

    pub(super) fn push_opaque_issue(
        ast_nodes: &mut Vec<SyntaxNode>,
        parent: AstNodeId,
        bounds: Range<usize>,
        issues: &mut Vec<ParseIssue>,
    ) {
        AstFactory::push_ast_node(
            ast_nodes,
            Some(parent),
            SyntaxNodeKind::Opaque,
            bounds.clone(),
            Vec::new(),
        );
        issues.push(AstFactory::unsupported_issue(
            UnsupportedConstructKind::Expression,
            bounds,
            LocalizedMessage::new(SYNTAX_ITEM_UNSUPPORTED),
        ));
    }

    pub(super) fn token_range(tokens: &[Token], start: usize, end: usize) -> Option<Range<usize>> {
        let first = tokens
            .iter()
            .position(|token| token.start() >= start && token.end() <= end)?;
        let after = tokens
            .iter()
            .enumerate()
            .skip(first)
            .find(|(_, token)| token.end() > end)
            .map_or(tokens.len(), |(index, _)| index);
        (first < after).then_some(first..after)
    }

    pub(super) fn top_level_token(
        tokens: &[Token],
        start: usize,
        end: usize,
        punctuation: &str,
    ) -> Option<usize> {
        let mut index = start;
        while index < end {
            if matches!(tokens[index].kind(), TokenKind::OpenDelimiter(_)) {
                if let Some(close) = Recovery::matching_delimiter(tokens, index) {
                    if close < end {
                        index = close + 1;
                        continue;
                    }
                }
            }
            if Recovery::is_punctuation(&tokens[index], punctuation) {
                return Some(index);
            }
            index += 1;
        }
        None
    }

    pub(super) fn push_pattern(
        input: PatternInput<'_>,
        ast_nodes: &mut Vec<SyntaxNode>,
    ) -> Option<AstNodeId> {
        let PatternInput {
            source,
            tokens,
            parent,
            start,
            end,
            bind_bare_identifiers,
            issues,
        } = input;
        let range = Self::token_range(tokens, start, end)?;
        if range.is_empty() {
            return None;
        }
        let parsed = if bind_bare_identifiers {
            PatternParser::parse_binding_range(tokens, range.clone())
        } else {
            PatternParser::parse_range(tokens, range.clone())
        };
        let syntax = match parsed {
            Ok(pattern) => pattern,
            Err(error) => return Self::push_pattern_error(error, issues),
        };
        let mut context = EmitContext {
            source,
            tokens,
            edition: None,
            ast_nodes,
            issues,
        };
        Some(Self::emit_pattern_syntax(&syntax, parent, &mut context))
    }

    pub(super) fn collect_parameter_pattern(
        input: ParameterPatternInput<'_>,
        ast_nodes: &mut Vec<SyntaxNode>,
    ) {
        let ParameterPatternInput {
            source,
            tokens,
            parent,
            parameter,
            issues,
        } = input;
        let start = parameter
            .attributes()
            .last()
            .map_or(parameter.start(), Attribute::end);
        let Some(range) = Self::token_range(tokens, start, parameter.end()) else {
            return;
        };
        let pattern_end = Self::top_level_token(tokens, range.start, range.end, ":")
            .map_or(parameter.end(), |index| tokens[index].start());
        let pattern_range = Self::token_range(tokens, tokens[range.start].start(), pattern_end)
            .unwrap_or(range.start..range.start);
        if pattern_range.is_empty() {
            return;
        }
        let parsed = if parameter.is_receiver() {
            PatternParser::parse_receiver_range(tokens, pattern_range)
        } else {
            PatternParser::parse_binding_range(tokens, pattern_range)
        };
        let syntax = match parsed {
            Ok(pattern) => pattern,
            Err(error) => {
                Self::push_pattern_error(error, issues);
                return;
            }
        };
        let mut context = EmitContext {
            source,
            tokens,
            edition: None,
            ast_nodes,
            issues,
        };
        Self::emit_pattern_syntax(&syntax, parent, &mut context);
    }

    fn push_pattern_error(error: SyntaxError, issues: &mut Vec<ParseIssue>) -> Option<AstNodeId> {
        let kind = if error.kind() == SyntaxErrorKind::UnclosedDelimiter {
            ParseIssueKind::UnclosedDelimiter
        } else {
            ParseIssueKind::UnexpectedToken
        };
        issues.push(AstFactory::parse_issue_with_boundary(
            kind,
            error.bounds(),
            LocalizedMessage::new(SYNTAX_ITEM_UNSUPPORTED),
            error.kind() == SyntaxErrorKind::UnclosedDelimiter,
        ));
        None
    }

    fn emit_pattern_syntax(
        syntax: &PatternSyntax,
        parent: AstNodeId,
        context: &mut EmitContext<'_>,
    ) -> AstNodeId {
        let pattern_id = AstFactory::push_data_node(
            context.ast_nodes,
            parent,
            SyntaxNodeKind::Pattern,
            syntax.bounds(),
            SyntaxNodeData::Pattern(syntax.clone()),
        );
        Self::set_pattern_attributes(syntax, pattern_id, context);
        Self::emit_pattern_children(syntax, pattern_id, context);
        pattern_id
    }

    fn emit_compatibility_nodes(
        syntax: &ExpressionSyntax,
        parent: AstNodeId,
        context: &mut EmitContext<'_>,
    ) -> AstNodeId {
        let mut attributes = Vec::new();
        for bounds in syntax.attributes() {
            if let Some(range) = Self::token_range(context.tokens, bounds.start, bounds.end) {
                let mut index = range.start;
                attributes.extend(AttributeParser::parse_attributes(
                    context.source,
                    context.tokens,
                    &mut index,
                    context.issues,
                ));
            }
        }
        let node_id = AstFactory::push_ast_node(
            context.ast_nodes,
            Some(parent),
            SyntaxNodeKind::Expression,
            syntax.bounds(),
            attributes,
        );
        AstFactory::set_ast_node_data(
            context.ast_nodes,
            node_id,
            SyntaxNodeData::Expression(syntax.clone()),
        );
        Self::emit_kind_children(syntax, node_id, context);
        node_id
    }

    fn validate_condition_edition(condition: &ExpressionSyntax, context: &mut EmitContext<'_>) {
        let Some(edition) = context.edition else {
            return;
        };
        if let Some(bounds) = EditionParser::invalid_let_condition(condition, edition) {
            context.issues.push(AstFactory::parse_issue(
                ParseIssueKind::UnexpectedToken,
                bounds,
                LocalizedMessage::new(SYNTAX_PARSE_ERROR_UNEXPECTED_TOKEN),
            ));
        }
    }

    fn emit_kind_children(
        syntax: &ExpressionSyntax,
        parent: AstNodeId,
        context: &mut EmitContext<'_>,
    ) {
        match syntax.kind() {
            ExpressionSyntaxKind::Path(path) => {
                TypeParser::emit_path_argument_macros(
                    path,
                    parent,
                    context.tokens,
                    context.ast_nodes,
                    context.issues,
                );
            }
            ExpressionSyntaxKind::Group(value) => {
                Self::emit_compatibility_nodes(value, parent, context);
            }
            ExpressionSyntaxKind::Tuple(values) => {
                for value in values {
                    Self::emit_compatibility_nodes(value, parent, context);
                }
            }
            ExpressionSyntaxKind::Array(values) => {
                for value in values {
                    Self::emit_compatibility_nodes(value, parent, context);
                }
            }
            ExpressionSyntaxKind::Repeat { value, count } => {
                Self::emit_compatibility_nodes(value, parent, context);
                Self::emit_compatibility_nodes(count, parent, context);
            }
            ExpressionSyntaxKind::Unary { operand: value, .. }
            | ExpressionSyntaxKind::Await(value)
            | ExpressionSyntaxKind::Try(value) => {
                Self::emit_compatibility_nodes(value, parent, context);
            }
            ExpressionSyntaxKind::Struct { path, fields } => {
                TypeParser::emit_path_argument_macros(
                    path,
                    parent,
                    context.tokens,
                    context.ast_nodes,
                    context.issues,
                );
                for field in fields {
                    if let Some(value) = field.value() {
                        let value = value.clone().with_attributes(field.attributes().to_vec());
                        Self::emit_compatibility_nodes(&value, parent, context);
                    }
                }
            }
            ExpressionSyntaxKind::Binary { left, right, .. }
            | ExpressionSyntaxKind::Assignment { left, right, .. } => {
                Self::emit_compatibility_nodes(left, parent, context);
                Self::emit_compatibility_nodes(right, parent, context);
            }
            ExpressionSyntaxKind::Range { start, end, .. } => {
                if let Some(value) = start {
                    Self::emit_compatibility_nodes(value, parent, context);
                }
                if let Some(value) = end {
                    Self::emit_compatibility_nodes(value, parent, context);
                }
            }
            ExpressionSyntaxKind::Call { callee, arguments } => {
                Self::emit_compatibility_nodes(callee, parent, context);
                for argument in arguments {
                    Self::emit_compatibility_nodes(argument, parent, context);
                }
            }
            ExpressionSyntaxKind::MethodCall {
                receiver,
                generic_arguments,
                arguments,
                ..
            } => {
                Self::emit_compatibility_nodes(receiver, parent, context);
                TypeParser::emit_generic_argument_macros(
                    generic_arguments,
                    parent,
                    context.tokens,
                    context.ast_nodes,
                    context.issues,
                );
                for argument in arguments {
                    Self::emit_compatibility_nodes(argument, parent, context);
                }
            }
            ExpressionSyntaxKind::Field { base, .. }
            | ExpressionSyntaxKind::TupleField { base, .. } => {
                Self::emit_compatibility_nodes(base, parent, context);
            }
            ExpressionSyntaxKind::Index { base, index } => {
                Self::emit_compatibility_nodes(base, parent, context);
                Self::emit_compatibility_nodes(index, parent, context);
            }
            ExpressionSyntaxKind::Cast { expression, target } => {
                Self::emit_compatibility_nodes(expression, parent, context);
                let type_id = AstFactory::push_data_node(
                    context.ast_nodes,
                    parent,
                    SyntaxNodeKind::Type,
                    target.bounds(),
                    SyntaxNodeData::Type(target.clone()),
                );
                TypeParser::emit_opaque_macros(
                    target,
                    type_id,
                    context.tokens,
                    context.ast_nodes,
                    context.issues,
                );
            }
            ExpressionSyntaxKind::Closure {
                parameters,
                return_type,
                body,
                ..
            } => {
                let closure_id = AstFactory::push_ast_node(
                    context.ast_nodes,
                    Some(parent),
                    SyntaxNodeKind::Closure,
                    syntax.bounds(),
                    Vec::new(),
                );
                for parameter in parameters {
                    let attributes = Self::attributes_from_ranges(parameter.attributes(), context);
                    let parameter_id = AstFactory::push_ast_node(
                        context.ast_nodes,
                        Some(closure_id),
                        SyntaxNodeKind::Parameter,
                        parameter.bounds(),
                        attributes,
                    );
                    AstFactory::set_ast_node_data(
                        context.ast_nodes,
                        parameter_id,
                        SyntaxNodeData::Parameter(false),
                    );
                    Self::emit_pattern(parameter.pattern(), parameter_id, context);
                    if let Some(ty) = parameter.ty() {
                        let type_id = AstFactory::push_data_node(
                            context.ast_nodes,
                            parameter_id,
                            SyntaxNodeKind::Type,
                            ty.bounds(),
                            SyntaxNodeData::Type(ty.clone()),
                        );
                        TypeParser::emit_opaque_macros(
                            ty,
                            type_id,
                            context.tokens,
                            context.ast_nodes,
                            context.issues,
                        );
                    }
                }
                if let Some(ty) = return_type {
                    let type_id = AstFactory::push_data_node(
                        context.ast_nodes,
                        closure_id,
                        SyntaxNodeKind::Type,
                        ty.bounds(),
                        SyntaxNodeData::Type(ty.clone()),
                    );
                    TypeParser::emit_opaque_macros(
                        ty,
                        type_id,
                        context.tokens,
                        context.ast_nodes,
                        context.issues,
                    );
                }
                Self::emit_compatibility_nodes(body, closure_id, context);
            }
            ExpressionSyntaxKind::Block { body, .. }
            | ExpressionSyntaxKind::LabelBlock { body, .. } => {
                Self::emit_block(body.clone(), parent, context);
            }
            ExpressionSyntaxKind::If {
                condition,
                then_block,
                else_expression,
            } => {
                Self::validate_condition_edition(condition, context);
                Self::emit_compatibility_nodes(condition, parent, context);
                Self::emit_block(then_block.clone(), parent, context);
                if let Some(expression) = else_expression {
                    Self::emit_compatibility_nodes(expression, parent, context);
                }
            }
            ExpressionSyntaxKind::Match { scrutinee, arms } => {
                Self::emit_compatibility_nodes(scrutinee, parent, context);
                for arm in arms {
                    let attributes = Self::attributes_from_ranges(arm.attributes(), context);
                    let arm_id = AstFactory::push_ast_node(
                        context.ast_nodes,
                        Some(parent),
                        SyntaxNodeKind::MatchArm,
                        arm.bounds(),
                        attributes,
                    );
                    if let Some(pattern) = arm.pattern() {
                        Self::emit_pattern(pattern, arm_id, context);
                    }
                    if let Some(guard) = arm.guard() {
                        Self::emit_compatibility_nodes(guard, arm_id, context);
                    }
                    Self::emit_compatibility_nodes(arm.body(), arm_id, context);
                }
            }
            ExpressionSyntaxKind::Loop { body, .. } => {
                Self::emit_block(body.clone(), parent, context);
            }
            ExpressionSyntaxKind::While {
                condition, body, ..
            } => {
                Self::validate_condition_edition(condition, context);
                Self::emit_compatibility_nodes(condition, parent, context);
                Self::emit_block(body.clone(), parent, context);
            }
            ExpressionSyntaxKind::Return(value) => {
                if let Some(value) = value {
                    Self::emit_compatibility_nodes(value, parent, context);
                }
            }
            ExpressionSyntaxKind::Break { value, .. } => {
                if let Some(value) = value {
                    Self::emit_compatibility_nodes(value, parent, context);
                }
            }
            ExpressionSyntaxKind::For {
                pattern,
                iterator,
                body,
                ..
            } => {
                Self::emit_pattern(pattern, parent, context);
                Self::emit_compatibility_nodes(iterator, parent, context);
                Self::emit_block(body.clone(), parent, context);
            }
            ExpressionSyntaxKind::Let { pattern, value } => {
                Self::emit_pattern(pattern, parent, context);
                Self::emit_compatibility_nodes(value, parent, context);
            }
            ExpressionSyntaxKind::Macro { path, input, .. } => {
                MacroInputSupport::emit_expression(path, syntax.bounds(), input, parent, context);
            }
            ExpressionSyntaxKind::Literal
            | ExpressionSyntaxKind::Underscore
            | ExpressionSyntaxKind::Continue(_) => {}
            ExpressionSyntaxKind::Error(error_kind) => {
                let bounds = syntax.bounds();
                if bounds.start < bounds.end {
                    AstFactory::push_ast_node(
                        context.ast_nodes,
                        Some(parent),
                        SyntaxNodeKind::Opaque,
                        bounds.clone(),
                        Vec::new(),
                    );
                }
                let details = match error_kind {
                    ExpressionErrorKind::UnexpectedToken => ParseIssueDetails {
                        kind: ParseIssueKind::UnexpectedToken,
                        code: ParseIssueCode::UnexpectedToken,
                        boundary_unknown: false,
                    },
                    ExpressionErrorKind::UnexpectedEof => ParseIssueDetails {
                        kind: ParseIssueKind::UnexpectedToken,
                        code: ParseIssueCode::UnexpectedEof,
                        boundary_unknown: false,
                    },
                    ExpressionErrorKind::UnclosedDelimiter => ParseIssueDetails {
                        kind: ParseIssueKind::UnclosedDelimiter,
                        code: ParseIssueCode::UnclosedDelimiter,
                        boundary_unknown: true,
                    },
                };
                context.issues.push(AstFactory::parse_issue_with_code(
                    details.kind,
                    details.code,
                    bounds,
                    LocalizedMessage::new(SYNTAX_ITEM_UNSUPPORTED),
                    details.boundary_unknown,
                ));
            }
            ExpressionSyntaxKind::Unsupported(_) => {
                AstFactory::push_ast_node(
                    context.ast_nodes,
                    Some(parent),
                    SyntaxNodeKind::Opaque,
                    syntax.bounds(),
                    Vec::new(),
                );
                context.issues.push(AstFactory::unsupported_issue(
                    UnsupportedConstructKind::Expression,
                    syntax.bounds(),
                    LocalizedMessage::new(SYNTAX_EXPRESSION_UNSUPPORTED),
                ));
            }
        }
    }

    fn emit_pattern(
        pattern: &PatternSyntax,
        parent: AstNodeId,
        context: &mut EmitContext<'_>,
    ) -> AstNodeId {
        let bounds = pattern.bounds();
        let pattern_id = AstFactory::push_data_node(
            context.ast_nodes,
            parent,
            SyntaxNodeKind::Pattern,
            bounds,
            SyntaxNodeData::Pattern(pattern.clone()),
        );
        Self::set_pattern_attributes(pattern, pattern_id, context);
        Self::emit_pattern_children(pattern, pattern_id, context);
        pattern_id
    }

    fn set_pattern_attributes(
        pattern: &PatternSyntax,
        pattern_id: AstNodeId,
        context: &mut EmitContext<'_>,
    ) {
        let attributes = Self::attributes_from_ranges(pattern.attributes(), context);
        AstFactory::set_ast_node_attributes(context.ast_nodes, pattern_id, attributes);
    }

    fn emit_pattern_children(
        pattern: &PatternSyntax,
        pattern_id: AstNodeId,
        context: &mut EmitContext<'_>,
    ) {
        match pattern.kind() {
            PatternSyntaxKind::Binding { subpattern, .. } => {
                if let Some(subpattern) = subpattern {
                    Self::emit_pattern(subpattern, pattern_id, context);
                }
            }
            PatternSyntaxKind::Reference { pattern, .. } => {
                Self::emit_pattern(pattern, pattern_id, context);
            }
            PatternSyntaxKind::Range { start, end, .. } => {
                for value in start.iter().chain(end.iter()) {
                    Self::emit_pattern(value, pattern_id, context);
                }
            }
            PatternSyntaxKind::Tuple(elements)
            | PatternSyntaxKind::Slice(elements)
            | PatternSyntaxKind::Or(elements) => {
                for element in elements {
                    Self::emit_pattern(element, pattern_id, context);
                }
            }
            PatternSyntaxKind::TupleStruct { path, elements } => {
                TypeParser::emit_path_argument_macros(
                    path,
                    pattern_id,
                    context.tokens,
                    context.ast_nodes,
                    context.issues,
                );
                for element in elements {
                    Self::emit_pattern(element, pattern_id, context);
                }
            }
            PatternSyntaxKind::Struct { path, fields, .. } => {
                TypeParser::emit_path_argument_macros(
                    path,
                    pattern_id,
                    context.tokens,
                    context.ast_nodes,
                    context.issues,
                );
                for field in fields {
                    if let Some(pattern) = field.pattern() {
                        let field_pattern_id = Self::emit_pattern(pattern, pattern_id, context);
                        let mut attributes =
                            Self::attributes_from_ranges(field.attributes(), context);
                        if let Some(field_pattern) = context.ast_nodes.get(field_pattern_id.index())
                        {
                            attributes.extend(field_pattern.attributes().iter().cloned());
                        }
                        AstFactory::set_ast_node_attributes(
                            context.ast_nodes,
                            field_pattern_id,
                            attributes,
                        );
                    }
                }
            }
            PatternSyntaxKind::Macro { path, input, .. } => {
                MacroInputSupport::emit_opaque(path, pattern.bounds(), input, pattern_id, context);
            }
            PatternSyntaxKind::Path(path) => {
                TypeParser::emit_path_argument_macros(
                    path,
                    pattern_id,
                    context.tokens,
                    context.ast_nodes,
                    context.issues,
                );
            }
            PatternSyntaxKind::Wildcard
            | PatternSyntaxKind::Literal(_)
            | PatternSyntaxKind::Rest => {}
        }
    }

    fn attributes_from_ranges(
        ranges: &[Range<usize>],
        context: &mut EmitContext<'_>,
    ) -> Vec<Attribute> {
        let mut attributes = Vec::new();
        for bounds in ranges {
            let Some(range) = Self::token_range(context.tokens, bounds.start, bounds.end) else {
                continue;
            };
            let mut index = range.start;
            attributes.extend(AttributeParser::parse_attributes(
                context.source,
                context.tokens,
                &mut index,
                context.issues,
            ));
        }
        attributes
    }

    fn emit_block(bounds: Range<usize>, parent: AstNodeId, context: &mut EmitContext<'_>) {
        if let Some(range) = Self::token_range(context.tokens, bounds.start, bounds.end) {
            if !range.is_empty() {
                AstFactory::push_ast_node(
                    context.ast_nodes,
                    Some(parent),
                    SyntaxNodeKind::Block,
                    bounds,
                    Vec::new(),
                );
            }
        }
    }
}
