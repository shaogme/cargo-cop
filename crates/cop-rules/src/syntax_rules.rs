use cop_analysis::{AnalysisSnapshot, CfgCondition, ModuleNode, SymbolKind};
use cop_config::ResolvedConfig;
use cop_model::{
    Diagnostic, LocalizedMessage, RuleId, CLOSURE_PARAMETER_LIMIT, DECLARATION_BEFORE_USE,
    FUNCTION_PARAMETER_LIMIT, NO_LINT_RELAXATION, TUPLE_ARITY,
};
use cop_syntax::{
    ClosureParameterSyntax, ExpressionSyntaxKind, ItemSyntaxKind, SyntaxNode, SyntaxNodeKind,
};

use crate::support::{PathReference, RuleDiagnosticAdapter, RuleReport, SyntaxSupport};

pub(super) struct SyntaxRuleSet;

impl SyntaxRuleSet {
    pub(super) fn run(
        snapshot: &AnalysisSnapshot,
        configuration: &ResolvedConfig,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let adapter = RuleDiagnosticAdapter::new(configuration);
        for module in snapshot.modules() {
            if module.is_order_only() {
                continue;
            }
            Self::lint_relaxation(module, diagnostics, &adapter);
            Self::syntax_shapes(module, diagnostics, &adapter);
            Self::declaration_order(module, snapshot, diagnostics, &adapter);
        }
    }

    fn lint_relaxation(
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        for attribute in module.lint_relaxations() {
            adapter.push(
                module,
                diagnostics,
                RuleReport::new(
                    RuleId::NoLintRelaxation,
                    LocalizedMessage::new(NO_LINT_RELAXATION),
                    Some(attribute.start()..attribute.end()),
                )
                .with_cfg_condition(attribute.condition().map(str::to_owned)),
            );
        }
    }

    fn syntax_shapes(
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        for item in module.item_nodes() {
            Self::check_shapes(item, module, diagnostics, adapter);
        }
    }

    fn check_shapes(
        root: &SyntaxNode,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        if root
            .item_syntax()
            .is_some_and(|data| matches!(data.kind(), ItemSyntaxKind::Module { .. }))
        {
            return;
        }
        SyntaxSupport::for_each_expression_syntax(module, root, &mut |expression| match expression
            .kind()
        {
            ExpressionSyntaxKind::Closure { parameters, .. } => {
                let parameter_count = parameters
                    .iter()
                    .filter(|parameter| {
                        let bounds = parameter.bounds();
                        module
                            .configured_ast()
                            .is_enabled_range(bounds.start, bounds.end)
                    })
                    .count();
                if parameter_count > 5 {
                    adapter.push(
                        module,
                        diagnostics,
                        RuleReport::new(
                            RuleId::ParameterLimit,
                            LocalizedMessage::new(CLOSURE_PARAMETER_LIMIT)
                                .with_number("actual", parameter_count as i64)
                                .with_number("maximum", 5),
                            Some(expression.bounds()),
                        )
                        .with_cfg_condition(closure_parameter_condition(module, parameters)),
                    );
                }
            }
            ExpressionSyntaxKind::Tuple(members) => {
                let arity = members
                    .iter()
                    .filter(|member| {
                        let bounds = member.bounds();
                        module
                            .configured_ast()
                            .is_enabled_range(bounds.start, bounds.end)
                    })
                    .count();
                if arity > 2 {
                    let bounds = expression.bounds();
                    adapter.push(
                        module,
                        diagnostics,
                        RuleReport::new(
                            RuleId::TupleArity,
                            LocalizedMessage::new(TUPLE_ARITY)
                                .with_number("actual", arity as i64)
                                .with_number("maximum", 2),
                            Some(bounds),
                        ),
                    );
                }
            }
            _ => {}
        });
        let output = module.parse_output();
        let mut pending = vec![root.id()];
        while let Some(id) = pending.pop() {
            let Some(node) = output.node(id) else {
                continue;
            };
            if node.id() != root.id()
                && node.item_syntax().is_some_and(|item_data| {
                    matches!(item_data.kind(), ItemSyntaxKind::Module { .. })
                })
            {
                continue;
            }
            if node.item_syntax().is_some_and(|item_data| {
                matches!(item_data.kind(), ItemSyntaxKind::Function { .. })
            }) {
                let parameter_count = node
                    .children()
                    .iter()
                    .filter_map(|id| output.node(*id))
                    .filter(|parameter| {
                        parameter.kind() == SyntaxNodeKind::Parameter
                            && module
                                .configured_ast()
                                .is_enabled_range(parameter.start(), parameter.end())
                    })
                    .count();
                if parameter_count > 5 {
                    adapter.push(
                        module,
                        diagnostics,
                        RuleReport::new(
                            RuleId::ParameterLimit,
                            LocalizedMessage::new(FUNCTION_PARAMETER_LIMIT)
                                .with_number("actual", parameter_count as i64)
                                .with_number("maximum", 5),
                            Some(node.start()..node.end()),
                        ),
                    );
                }
            }
            pending.extend(node.children().iter().rev().copied());
        }
    }

    fn declaration_order(
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        for item in module.item_nodes() {
            SyntaxSupport::for_each_path_reference(module, item, false, &mut |reference| {
                check_ordered_path(reference, module, snapshot, diagnostics, adapter);
            });
            if item
                .item_syntax()
                .is_some_and(|data| matches!(data.kind(), ItemSyntaxKind::Use))
            {
                let Some(tree) = SyntaxSupport::use_tree_node_for_item(module, item) else {
                    continue;
                };
                for leaf in SyntaxSupport::use_leaves(module, tree) {
                    if !leaf.glob {
                        check_ordered_path(
                            &PathReference {
                                segments: leaf.path,
                                start: leaf.start,
                                end: leaf.end,
                                is_macro: false,
                            },
                            module,
                            snapshot,
                            diagnostics,
                            adapter,
                        );
                    }
                }
            }
        }
    }
}

fn closure_parameter_condition(
    module: &ModuleNode,
    parameters: &[ClosureParameterSyntax],
) -> Option<String> {
    let mut unconditional = 0usize;
    let mut conditions = Vec::new();
    for parameter in parameters {
        let bounds = parameter.bounds();
        if !module
            .configured_ast()
            .is_enabled_range(bounds.start, bounds.end)
        {
            continue;
        }
        match module
            .configured_ast()
            .condition_for_span(bounds.start, bounds.end)
        {
            Some(condition) => conditions.push(CfgCondition::parse(condition).ok()?),
            None => unconditional += 1,
        }
    }
    let required = 6usize.saturating_sub(unconditional);
    if required == 0 {
        return None;
    }
    if conditions.len() < required || conditions.len() > 12 {
        return None;
    }
    let mut at_least = vec![CfgCondition::any([]); required + 1];
    at_least[0] = CfgCondition::all([]);
    for condition in conditions {
        for count in (1..=required).rev() {
            at_least[count] = CfgCondition::any([
                at_least[count].clone(),
                CfgCondition::all([at_least[count - 1].clone(), condition.clone()]),
            ]);
        }
    }
    at_least.get(required).map(CfgCondition::normalized)
}

fn check_ordered_path(
    reference: &PathReference,
    module: &ModuleNode,
    snapshot: &AnalysisSnapshot,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) {
    if !module
        .configured_ast()
        .is_enabled_range(reference.start, reference.end)
        || !module.configured_ast().is_supported_range(
            module.parse_output(),
            reference.start,
            reference.end,
        )
    {
        return;
    }
    let reference_condition = module
        .configured_ast()
        .condition_for_span(reference.start, reference.end);
    let resolution = snapshot.symbols().resolve_path_at(
        module,
        &reference.segments,
        Some(reference.start..reference.end),
    );
    let Some(symbol) = resolution.symbol().filter(|symbol| {
        symbol.namespace() == module.namespace()
            && symbol.file_path() == module.file_path()
            && is_ordered_declaration(symbol.kind())
            && SyntaxSupport::conditions_may_coexist(reference_condition, symbol.cfg_condition())
    }) else {
        return;
    };
    let Some(declaration_span) = symbol.span() else {
        return;
    };
    if reference.start >= declaration_span.byte_range().start {
        return;
    }
    adapter.push(
        module,
        diagnostics,
        RuleReport::new(
            RuleId::DeclarationBeforeUse,
            LocalizedMessage::new(DECLARATION_BEFORE_USE).with_text("name", symbol.name()),
            Some(reference.start..reference.end),
        ),
    );
}

fn is_ordered_declaration(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Struct
            | SymbolKind::Enum
            | SymbolKind::Union
            | SymbolKind::TypeAlias
            | SymbolKind::Trait
            | SymbolKind::Constant
            | SymbolKind::Static
    )
}
