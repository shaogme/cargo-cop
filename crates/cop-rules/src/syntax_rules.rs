use crate::support::{RuleDiagnosticAdapter, RuleReport, SyntaxSupport};
use cop_analysis::{
    AnalysisReference, AnalysisSnapshot, CfgCondition, ModuleNode, ReferenceKind,
    ShapeParameterFact, SymbolKind, SyntaxShapeKind,
};
use cop_config::ResolvedConfig;
use cop_model::{
    Diagnostic, LocalizedMessage, RuleId, CLOSURE_PARAMETER_LIMIT, DECLARATION_BEFORE_USE,
    FUNCTION_PARAMETER_LIMIT, NO_LINT_RELAXATION, TUPLE_ARITY,
};

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
            Self::syntax_shapes(module, snapshot, diagnostics, &adapter);
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
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        let Some(syntax) = snapshot.facts().syntax_for_module(module) else {
            return;
        };
        for shape in syntax.shapes() {
            let bounds = shape.bounds();
            match shape.kind() {
                SyntaxShapeKind::ClosureParameters(parameters) => {
                    let parameter_count = parameters
                        .iter()
                        .filter(|parameter| parameter.is_enabled())
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
                                Some(bounds),
                            )
                            .with_cfg_condition(closure_parameter_condition(parameters)),
                        );
                    }
                }
                SyntaxShapeKind::TupleArity(arity) => {
                    if *arity > 2 {
                        adapter.push(
                            module,
                            diagnostics,
                            RuleReport::new(
                                RuleId::TupleArity,
                                LocalizedMessage::new(TUPLE_ARITY)
                                    .with_number("actual", *arity as i64)
                                    .with_number("maximum", 2),
                                Some(bounds),
                            ),
                        );
                    }
                }
                SyntaxShapeKind::FunctionParameters(parameter_count) => {
                    if *parameter_count > 5 {
                        adapter.push(
                            module,
                            diagnostics,
                            RuleReport::new(
                                RuleId::ParameterLimit,
                                LocalizedMessage::new(FUNCTION_PARAMETER_LIMIT)
                                    .with_number("actual", *parameter_count as i64)
                                    .with_number("maximum", 5),
                                Some(bounds),
                            ),
                        );
                    }
                }
            }
        }
    }

    fn declaration_order(
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        for reference in snapshot.facts().references_for_module(module) {
            if matches!(reference.kind(), ReferenceKind::Path | ReferenceKind::Call)
                || (reference.kind() == ReferenceKind::Use && !reference.is_glob())
            {
                check_ordered_path(reference, module, diagnostics, adapter);
            }
        }
    }
}

fn closure_parameter_condition(parameters: &[ShapeParameterFact]) -> Option<String> {
    let mut unconditional = 0usize;
    let mut conditions = Vec::new();
    for parameter in parameters {
        if !parameter.is_enabled() {
            continue;
        }
        match parameter.condition() {
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
    reference: &AnalysisReference,
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) {
    let bounds = reference.bounds();
    if !module
        .configured_ast()
        .is_enabled_range(bounds.start, bounds.end)
        || !module.configured_ast().is_supported_range(
            module.parse_output(),
            bounds.start,
            bounds.end,
        )
    {
        return;
    }
    let reference_condition = reference.condition();
    let resolution = reference.resolution();
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
    if bounds.start >= declaration_span.byte_range().start {
        return;
    }
    adapter.push(
        module,
        diagnostics,
        RuleReport::new(
            RuleId::DeclarationBeforeUse,
            LocalizedMessage::new(DECLARATION_BEFORE_USE).with_text("name", symbol.name()),
            Some(bounds),
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
