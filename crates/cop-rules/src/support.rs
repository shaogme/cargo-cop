mod path_syntax;

use std::ops::Range;

use cop_analysis::{CfgCondition, CfgDecision, ModuleNode};
use cop_config::ResolvedConfig;
use cop_model::{Diagnostic, LocalizedMessage, RuleId, SourceSpan};
use cop_syntax::{
    AstNodeId, Attribute, ExpressionSyntax, ItemSyntaxKind, ParseOutput, SyntaxNode,
    SyntaxNodeKind, TriviaKind, VisibilityKind, WherePredicateKind,
};

pub(super) struct RuleReport {
    rule_id: RuleId,
    message: LocalizedMessage,
    bounds: Option<Range<usize>>,
    span: Option<SourceSpan>,
    cfg_condition: Option<String>,
}

pub(super) struct RuleDiagnosticAdapter<'a> {
    configuration: &'a ResolvedConfig,
}

pub(super) struct UseLeaf {
    pub(super) path: Vec<String>,
    pub(super) alias: Option<String>,
    pub(super) glob: bool,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) condition: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PathReference {
    pub(super) segments: Vec<String>,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) is_macro: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpacingDefinitionKind {
    Struct,
    Enum,
    Union,
    TypeAlias,
    Trait,
    Constant,
    Static,
    Function,
}

pub(super) struct SyntaxSupport;

impl SyntaxSupport {
    pub(super) fn common_import_prefix(left: &[UseLeaf], right: &[UseLeaf]) -> Option<String> {
        let mut best_prefix: Option<Vec<String>> = None;
        for left_leaf in left {
            if left_leaf.alias.is_some() || left_leaf.glob {
                continue;
            }
            for right_leaf in right {
                if right_leaf.alias.is_some() || right_leaf.glob {
                    continue;
                }
                let prefix_length = Self::common_prefix_len(&left_leaf.path, &right_leaf.path);
                if prefix_length == 0
                    || prefix_length >= left_leaf.path.len()
                    || prefix_length >= right_leaf.path.len()
                    || best_prefix
                        .as_ref()
                        .is_some_and(|prefix| prefix.len() >= prefix_length)
                {
                    continue;
                }
                best_prefix = Some(left_leaf.path[..prefix_length].to_vec());
            }
        }
        best_prefix.map(|prefix| prefix.join("::"))
    }

    fn common_prefix_len(left: &[String], right: &[String]) -> usize {
        left.iter()
            .zip(right)
            .take_while(|(left_segment, right_segment)| left_segment == right_segment)
            .count()
    }
}

impl RuleReport {
    pub(super) fn new(
        rule_id: RuleId,
        message: LocalizedMessage,
        bounds: Option<Range<usize>>,
    ) -> Self {
        Self {
            rule_id,
            message,
            bounds,
            span: None,
            cfg_condition: None,
        }
    }

    pub(super) fn at_span(rule_id: RuleId, message: LocalizedMessage, span: SourceSpan) -> Self {
        Self {
            rule_id,
            message,
            bounds: None,
            span: Some(span),
            cfg_condition: None,
        }
    }

    pub(super) fn with_cfg_condition(mut self, condition: Option<String>) -> Self {
        self.cfg_condition = condition;
        self
    }
}

impl RuleDiagnosticAdapter<'_> {
    pub(super) fn new(configuration: &ResolvedConfig) -> RuleDiagnosticAdapter<'_> {
        RuleDiagnosticAdapter { configuration }
    }

    pub(super) fn push(
        &self,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        report: RuleReport,
    ) {
        let setting = self.configuration.rule(report.rule_id);
        if !setting.enabled() {
            return;
        }
        let explicit_condition = report.cfg_condition.clone();
        let cfg_bounds = report
            .bounds
            .clone()
            .or_else(|| report.span.as_ref().map(|span| span.byte_range()));
        if cfg_bounds.as_ref().is_some_and(|bounds| {
            module
                .configured_ast()
                .node_for_range(bounds.start, bounds.end)
                .is_some_and(|node| node.decision() == CfgDecision::False)
        }) {
            return;
        }
        let mut diagnostic = Diagnostic::rule(report.rule_id, setting.severity(), report.message)
            .with_package(module.package_name())
            .with_package_manifest_path(module.package_manifest_path())
            .with_target(module.target_name())
            .with_target_kind(module.target_kind().to_vec())
            .with_namespace(namespace_name(module.namespace()));
        if let Some(condition) = explicit_condition
            .as_deref()
            .or_else(|| {
                cfg_bounds.as_ref().and_then(|bounds| {
                    module
                        .configured_ast()
                        .node_for_range(bounds.start, bounds.end)
                        .and_then(|node| node.effective_condition())
                })
            })
            .or(module.cfg_condition())
        {
            diagnostic = diagnostic.with_cfg_condition(condition);
        }
        if let Some(span) = report.span {
            diagnostic = diagnostic.with_span(span);
        } else if let Some(bounds) = report.bounds {
            if let Ok(span) = module
                .parse_output()
                .source()
                .span(bounds.start, bounds.end)
            {
                diagnostic = diagnostic.with_span(span);
            }
        }
        diagnostics.push(diagnostic);
    }
}

impl SyntaxSupport {
    pub(super) fn conditions_may_coexist(left: Option<&str>, right: Option<&str>) -> bool {
        let Some(left) = left else {
            return true;
        };
        let Some(right) = right else {
            return true;
        };
        match (CfgCondition::parse(left), CfgCondition::parse(right)) {
            (Ok(left), Ok(right)) => CfgCondition::may_coexist(&left, &right),
            _ => true,
        }
    }

    pub(super) fn expected_spacing_blank_lines(
        previous: &SyntaxNode,
        current: &SyntaxNode,
    ) -> Option<usize> {
        let previous_kind = Self::spacing_definition_kind(previous.item_syntax()?.kind())?;
        let current_kind = Self::spacing_definition_kind(current.item_syntax()?.kind())?;
        Some(match (previous_kind, current_kind) {
            (SpacingDefinitionKind::Constant, SpacingDefinitionKind::Constant)
            | (SpacingDefinitionKind::Static, SpacingDefinitionKind::Static)
            | (SpacingDefinitionKind::TypeAlias, SpacingDefinitionKind::TypeAlias) => 0,
            _ => 1,
        })
    }

    pub(super) fn is_non_private(item: &SyntaxNode) -> bool {
        item.item_syntax()
            .is_some_and(|data| !matches!(data.visibility().kind(), VisibilityKind::Private))
    }

    pub(super) fn is_cfg_test(attributes: &[Attribute]) -> bool {
        attributes.iter().any(Attribute::is_cfg_test)
    }

    pub(super) fn for_each_path_reference(
        module: &ModuleNode,
        item: &SyntaxNode,
        include_macros: bool,
        visitor: &mut dyn FnMut(&PathReference),
    ) {
        if item
            .item_syntax()
            .is_some_and(|data| matches!(data.kind(), ItemSyntaxKind::Module { .. }))
        {
            return;
        }
        let output = module.parse_output();
        let mut path_references = Vec::new();
        let mut typed_ranges = Vec::new();
        let mut pending = vec![item.id()];
        while let Some(id) = pending.pop() {
            let Some(node) = output.node(id) else {
                continue;
            };
            if node.id() != item.id()
                && node
                    .item_syntax()
                    .is_some_and(|data| matches!(data.kind(), ItemSyntaxKind::Module { .. }))
            {
                continue;
            }
            if let Some(expression) = node.expression_syntax() {
                typed_ranges.push(node.start()..node.end());
                path_syntax::PathSyntaxCollector::collect_expression(
                    expression,
                    &mut path_references,
                );
            }
            if let Some(syntax) = node.impl_syntax() {
                if let Some(path) = syntax.trait_path() {
                    typed_ranges.push(path.bounds());
                    path_syntax::PathSyntaxCollector::collect_path(path, &mut path_references);
                }
            }
            if let Some(syntax) = node.type_syntax() {
                typed_ranges.push(node.start()..node.end());
                path_syntax::PathSyntaxCollector::collect_type(syntax, &mut path_references);
            }
            if let Some(syntax) = node.pattern_syntax() {
                typed_ranges.push(node.start()..node.end());
                path_syntax::PathSyntaxCollector::collect_pattern(syntax, &mut path_references);
            }
            if node.generic_parameter_syntax().is_some() {
                typed_ranges.push(node.start()..node.end());
                if let Some(parameter) = node.generic_parameter_syntax() {
                    for bound in parameter.bounds() {
                        path_syntax::PathSyntaxCollector::collect_trait_bound(
                            bound,
                            &mut path_references,
                        );
                    }
                    if let Some(ty) = parameter.declared_type() {
                        path_syntax::PathSyntaxCollector::collect_type(ty, &mut path_references);
                    }
                    if let Some(ty) = parameter.default_type() {
                        path_syntax::PathSyntaxCollector::collect_type(ty, &mut path_references);
                    }
                }
            }
            if let Some(predicate) = node.where_predicate_syntax() {
                typed_ranges.push(node.start()..node.end());
                match predicate.kind() {
                    WherePredicateKind::Lifetime { .. } => {}
                    WherePredicateKind::Type {
                        bounded_type,
                        bounds,
                        ..
                    } => {
                        path_syntax::PathSyntaxCollector::collect_type(
                            bounded_type,
                            &mut path_references,
                        );
                        for bound in bounds {
                            path_syntax::PathSyntaxCollector::collect_trait_bound(
                                bound,
                                &mut path_references,
                            );
                        }
                    }
                }
            }
            pending.extend(node.children().iter().rev().copied());
        }
        if include_macros {
            for node in output.syntax_nodes().iter().filter(|node| {
                node.kind() == SyntaxNodeKind::Macro
                    && is_descendant_of(output, node, item.id())
                    && !typed_ranges
                        .iter()
                        .any(|range| range.start <= node.start() && node.end() <= range.end)
            }) {
                if let Some(invocation) = node.macro_invocation() {
                    let bounds = invocation.path().bounds();
                    path_references.push(PathReference {
                        segments: path_syntax::PathSyntaxCollector::path_segments(
                            invocation.path(),
                        ),
                        start: bounds.start,
                        end: bounds.end,
                        is_macro: true,
                    });
                }
            }
        }
        path_references.sort_unstable_by_key(|path| (path.start, path.end));
        path_references.dedup_by(|left, right| {
            left.start == right.start
                && left.end == right.end
                && left.segments == right.segments
                && left.is_macro == right.is_macro
        });
        for path in &path_references {
            if path.is_macro && !include_macros {
                continue;
            }
            if module
                .configured_ast()
                .node_for_range(path.start, path.end)
                .is_some_and(|node| node.decision() == CfgDecision::False)
            {
                continue;
            }
            visitor(path);
        }
    }

    pub(super) fn for_each_expression_syntax(
        module: &ModuleNode,
        item: &SyntaxNode,
        visitor: &mut dyn FnMut(&ExpressionSyntax),
    ) {
        let output = module.parse_output();
        let mut pending = vec![item.id()];
        while let Some(id) = pending.pop() {
            let Some(node) = output.node(id) else {
                continue;
            };
            if node.id() != item.id()
                && node
                    .item_syntax()
                    .is_some_and(|data| matches!(data.kind(), ItemSyntaxKind::Module { .. }))
            {
                continue;
            }
            if let Some(expression) = node.expression_syntax() {
                let bounds = expression.bounds();
                if !module
                    .configured_ast()
                    .node_for_range(bounds.start, bounds.end)
                    .is_some_and(|configured| configured.decision() == CfgDecision::False)
                    && module
                        .configured_ast()
                        .is_supported_range(output, bounds.start, bounds.end)
                {
                    visitor(expression);
                }
            }
            pending.extend(node.children().iter().rev().copied());
        }
    }

    pub(super) fn child_item_nodes_for_node<'a>(
        module: &'a ModuleNode,
        root: &SyntaxNode,
    ) -> Vec<&'a SyntaxNode> {
        let output = module.parse_output();
        root.children()
            .iter()
            .filter_map(|id| output.node(*id))
            .find(|node| node.kind() == SyntaxNodeKind::Scope)
            .map(|scope| {
                scope
                    .children()
                    .iter()
                    .filter_map(|id| output.node(*id))
                    .filter(|node| node.item_syntax().is_some())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(super) fn use_leaves(module: &ModuleNode, tree: &SyntaxNode) -> Vec<UseLeaf> {
        let mut leaves = Vec::new();
        Self::collect_use_leaves(module, tree, &[], &mut leaves);
        leaves
    }

    pub(super) fn use_tree_node_for_item<'a>(
        module: &'a ModuleNode,
        item: &SyntaxNode,
    ) -> Option<&'a SyntaxNode> {
        let output = module.parse_output();
        item.children()
            .iter()
            .filter_map(|id| output.node(*id))
            .find(|node| node.kind() == SyntaxNodeKind::UseTree)
    }

    pub(super) fn blank_lines_between(
        output: &ParseOutput,
        previous_end: usize,
        next_start: usize,
    ) -> usize {
        let source = output.source();
        let Some((end_line, _)) = source.line_column(previous_end) else {
            return 0;
        };
        let Some((start_line, _)) = source.line_column(next_start) else {
            return 0;
        };
        if start_line <= end_line + 1 {
            return 0;
        }
        let comments = output
            .tokens()
            .iter()
            .flat_map(|token| token.leading_trivia())
            .chain(output.trailing_trivia())
            .filter(|trivia| {
                matches!(
                    trivia.kind(),
                    TriviaKind::LineComment | TriviaKind::BlockComment
                )
            })
            .map(|trivia| trivia.start()..trivia.end())
            .collect::<Vec<_>>();
        (end_line + 1..start_line)
            .filter(|line| {
                let Some(range) = source.line_range(*line) else {
                    return false;
                };
                let contains_comment = comments.iter().any(|comment| {
                    if range.is_empty() {
                        comment.start <= range.start && comment.end > range.start
                    } else {
                        comment.start < range.end && comment.end > range.start
                    }
                });
                !contains_comment
                    && source
                        .slice(range.start, range.end)
                        .is_some_and(|text| text.chars().all(char::is_whitespace))
            })
            .count()
    }

    fn collect_use_leaves(
        module: &ModuleNode,
        tree: &SyntaxNode,
        prefix: &[String],
        leaves: &mut Vec<UseLeaf>,
    ) {
        let start = tree.start();
        let end = tree.end();
        let Some(configured) = module.configured_ast().node(tree.id()) else {
            return;
        };
        if configured.decision() == CfgDecision::False {
            return;
        }
        let Some(tree_data) = tree.use_tree() else {
            return;
        };
        let mut path = prefix.to_vec();
        path.extend(tree_data.path().iter().cloned());
        let children = tree
            .children()
            .iter()
            .filter_map(|id| module.parse_output().node(*id))
            .filter(|node| node.kind() == SyntaxNodeKind::UseTree)
            .collect::<Vec<_>>();
        if children.is_empty() {
            if !module
                .configured_ast()
                .is_supported_range(module.parse_output(), start, end)
            {
                return;
            }
            leaves.push(UseLeaf {
                path,
                alias: tree_data.alias().map(str::to_owned),
                glob: tree_data.is_glob(),
                start,
                end,
                condition: configured.effective_condition().map(str::to_owned),
            });
            return;
        }
        for child in children {
            Self::collect_use_leaves(module, child, &path, leaves);
        }
    }

    fn spacing_definition_kind(kind: &ItemSyntaxKind) -> Option<SpacingDefinitionKind> {
        match kind {
            ItemSyntaxKind::Struct(_) => Some(SpacingDefinitionKind::Struct),
            ItemSyntaxKind::Enum(_) => Some(SpacingDefinitionKind::Enum),
            ItemSyntaxKind::Union(_) => Some(SpacingDefinitionKind::Union),
            ItemSyntaxKind::TypeAlias(_) => Some(SpacingDefinitionKind::TypeAlias),
            ItemSyntaxKind::Trait(_) => Some(SpacingDefinitionKind::Trait),
            ItemSyntaxKind::Constant(_) => Some(SpacingDefinitionKind::Constant),
            ItemSyntaxKind::Static(_) => Some(SpacingDefinitionKind::Static),
            ItemSyntaxKind::Function { .. } => Some(SpacingDefinitionKind::Function),
            _ => None,
        }
    }
}

fn is_descendant_of(output: &ParseOutput, node: &SyntaxNode, root: AstNodeId) -> bool {
    let mut parent = node.parent();
    while let Some(id) = parent {
        if id == root {
            return true;
        }
        parent = output.node(id).and_then(SyntaxNode::parent);
    }
    node.id() == root
}

fn namespace_name(namespace: &[String]) -> String {
    if namespace.is_empty() {
        "crate".to_owned()
    } else {
        format!("crate::{}", namespace.join("::"))
    }
}
