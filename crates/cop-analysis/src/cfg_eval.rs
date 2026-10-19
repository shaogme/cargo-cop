use std::ops::Range;

use cop_cargo::TargetContext;
use cop_cfg::{CfgCondition, CfgContext, CfgEvaluator as SharedCfgEvaluator, CfgExpression};
use cop_syntax::{
    Attribute, AttributeMeta, AttributeMetaArgument, AttributeMetaKind, AttributeMetaValueKind,
    LiteralKind, SourceFile,
};

pub use cop_cfg::CfgDecision;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UnknownCondition {
    pub(crate) condition: String,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConditionalAttribute {
    pub(crate) condition: Option<String>,
    pub(crate) decision: CfgDecision,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConditionalPathAttribute {
    pub(crate) condition: Option<String>,
    pub(crate) decision: CfgDecision,
    pub(crate) path: Option<String>,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct AttributeEffects {
    pub(crate) decision: Option<CfgDecision>,
    pub(crate) path: Option<String>,
    pub(crate) path_unknown: bool,
    pub(crate) conditions: Vec<String>,
    pub(crate) unknown: Vec<UnknownCondition>,
    pub(crate) lint_relaxations: Vec<ConditionalAttribute>,
    pub(crate) path_attributes: Vec<ConditionalPathAttribute>,
    pub(crate) malformed: bool,
    pub(crate) malformed_ranges: Vec<Range<usize>>,
}

struct AttributeParseContext<'a> {
    effects: &'a mut AttributeEffects,
    guards: Vec<CfgCondition>,
    depth: usize,
    path_only: bool,
}

pub(crate) struct CfgEvaluator<'a> {
    target: &'a TargetContext,
}

impl<'a> CfgEvaluator<'a> {
    pub(crate) fn new(target: &'a TargetContext) -> Self {
        Self { target }
    }

    pub(crate) fn evaluate_condition(&self, condition: &str) -> CfgDecision {
        let Ok(expression) = CfgExpression::parse(condition) else {
            return CfgDecision::Unknown;
        };
        let context = self.cfg_context();
        SharedCfgEvaluator::new(&context).evaluate(&expression)
    }

    pub(crate) fn normalized_condition(&self, condition: &str) -> String {
        CfgExpression::parse(condition)
            .map(|expression| expression.normalized())
            .unwrap_or_else(|_| condition.trim().to_owned())
    }

    pub(crate) fn evaluate_attributes(
        &self,
        attributes: &[Attribute],
        source: &SourceFile,
    ) -> AttributeEffects {
        let mut effects = AttributeEffects::default();
        for attribute in attributes {
            if source.slice(attribute.start(), attribute.end()).is_none() {
                record_malformed(&mut effects, attribute.start()..attribute.end());
                continue;
            }
            let mut context = AttributeParseContext {
                effects: &mut effects,
                guards: Vec::new(),
                depth: 0,
                path_only: false,
            };
            self.apply_attribute(attribute.meta(), &mut context);
        }
        effects
    }

    pub(crate) fn dependency_condition(&self, condition: &str) -> CfgDecision {
        let condition = condition.trim();
        if !condition.starts_with("cfg(") {
            return if condition == self.target.target_triple() {
                CfgDecision::True
            } else {
                CfgDecision::False
            };
        }
        self.evaluate_condition(condition)
    }

    fn apply_attribute(&self, meta: &AttributeMeta, context: &mut AttributeParseContext<'_>) {
        let bounds = meta.bounds();
        if context.depth > 16 {
            record_malformed(context.effects, bounds);
            return;
        }
        let name = meta.path().join("::");
        if context.path_only && !matches!(name.as_str(), "cfg_attr" | "path") {
            return;
        }
        match name.as_str() {
            "cfg" => {
                let Some(expression) = cfg_attribute_expression(meta) else {
                    record_malformed(context.effects, bounds);
                    return;
                };
                let condition =
                    guarded_condition(&context.guards, CfgCondition::from_expression(expression));
                let condition_text = condition.normalized();
                let decision = self.evaluate_cfg_condition(&condition);
                context.effects.decision = Some(
                    context
                        .effects
                        .decision
                        .unwrap_or(CfgDecision::True)
                        .and(decision),
                );
                context.effects.conditions.push(condition_text.clone());
                if decision == CfgDecision::Unknown {
                    context.effects.unknown.push(UnknownCondition {
                        condition: condition_text,
                        start: bounds.start,
                        end: bounds.end,
                    });
                }
            }
            "cfg_attr" => self.apply_cfg_attr(meta, context),
            "path" => {
                let decision = self.evaluate_guards(&context.guards);
                let path = string_value(meta);
                context
                    .effects
                    .path_attributes
                    .push(ConditionalPathAttribute {
                        condition: guard_condition(&context.guards)
                            .map(|condition| condition.normalized()),
                        decision,
                        path: path.clone(),
                        start: bounds.start,
                        end: bounds.end,
                    });
                match (decision, path) {
                    (CfgDecision::True, Some(path)) if context.effects.path.is_none() => {
                        context.effects.path = Some(path);
                    }
                    (CfgDecision::True, Some(_)) | (CfgDecision::True, None) => {
                        record_malformed(context.effects, bounds);
                    }
                    (CfgDecision::False, _) => {}
                    (CfgDecision::Unknown, Some(_)) => {
                        context.effects.path_unknown = true;
                        self.record_unknown_guard(&context.guards, bounds, context.effects);
                    }
                    (CfgDecision::Unknown, None) => {
                        record_malformed(context.effects, bounds);
                    }
                }
            }
            "allow" => {
                let decision = self.evaluate_guards(&context.guards);
                if decision != CfgDecision::False {
                    context.effects.lint_relaxations.push(ConditionalAttribute {
                        condition: guard_condition(&context.guards).map(|value| value.normalized()),
                        decision,
                        start: bounds.start,
                        end: bounds.end,
                    });
                    if decision == CfgDecision::Unknown {
                        self.record_unknown_guard(&context.guards, bounds, context.effects);
                    }
                }
            }
            _ => {}
        }
    }

    fn apply_cfg_attr(&self, meta: &AttributeMeta, context: &mut AttributeParseContext<'_>) {
        let AttributeMetaKind::List(arguments) = meta.kind() else {
            if !context.path_only {
                record_malformed(context.effects, meta.bounds());
            }
            return;
        };
        let Some(predicate_argument) = arguments.first() else {
            if !context.path_only {
                record_malformed(context.effects, meta.bounds());
            }
            return;
        };
        let Some(predicate) = predicate_argument.meta() else {
            if !context.path_only {
                record_malformed(context.effects, predicate_argument.bounds());
            }
            return;
        };
        if arguments.len() < 2 {
            if !context.path_only {
                record_malformed(context.effects, meta.bounds());
            }
            return;
        }
        let Some(predicate_expression) = cfg_expression(predicate) else {
            if !context.path_only {
                record_malformed(context.effects, predicate.bounds());
            }
            return;
        };
        let mut nested_guards = context.guards.clone();
        nested_guards.push(CfgCondition::from_expression(predicate_expression));
        let path_only =
            context.path_only || self.evaluate_guards(&nested_guards) == CfgDecision::False;
        let mut nested_context = AttributeParseContext {
            effects: &mut *context.effects,
            guards: nested_guards,
            depth: context.depth + 1,
            path_only,
        };
        for argument in arguments.iter().skip(1) {
            if let Some(nested) = argument.meta() {
                self.apply_attribute(nested, &mut nested_context);
            } else if !nested_context.path_only {
                record_malformed(nested_context.effects, argument.bounds());
            }
        }
    }

    fn cfg_context(&self) -> CfgContext<'_> {
        CfgContext::new(
            self.target.cfg_values(),
            Some(self.target.features().enabled()),
        )
        .with_test(Some(self.target.cfg_test()))
        .with_proc_macro(Some(
            self.target
                .crate_types()
                .iter()
                .any(|kind| kind == "proc-macro"),
        ))
    }

    fn evaluate_cfg_condition(&self, condition: &CfgCondition) -> CfgDecision {
        SharedCfgEvaluator::new(&self.cfg_context()).evaluate_condition(condition)
    }

    fn evaluate_guards(&self, guards: &[CfgCondition]) -> CfgDecision {
        guard_condition(guards)
            .as_ref()
            .map_or(CfgDecision::True, |condition| {
                self.evaluate_cfg_condition(condition)
            })
    }

    fn record_unknown_guard(
        &self,
        guards: &[CfgCondition],
        bounds: Range<usize>,
        effects: &mut AttributeEffects,
    ) {
        let Some(condition) = guard_condition(guards) else {
            return;
        };
        effects.unknown.push(UnknownCondition {
            condition: condition.normalized(),
            start: bounds.start,
            end: bounds.end,
        });
    }
}

fn cfg_attribute_expression(meta: &AttributeMeta) -> Option<CfgExpression> {
    let AttributeMetaKind::List(arguments) = meta.kind() else {
        return None;
    };
    let [AttributeMetaArgument::Meta(expression)] = arguments.as_slice() else {
        return None;
    };
    cfg_expression(expression)
}

fn cfg_expression(meta: &AttributeMeta) -> Option<CfgExpression> {
    let name = meta.path().join("::");
    let range = meta.bounds();
    match meta.kind() {
        AttributeMetaKind::Word => Some(CfgExpression::Flag { name, range }),
        AttributeMetaKind::NameValue(value) => match value.kind() {
            AttributeMetaValueKind::Literal {
                kind: LiteralKind::String,
                decoded_string: Some(value),
                ..
            } => Some(CfgExpression::KeyValue {
                name,
                value: value.clone(),
                range,
            }),
            _ => None,
        },
        AttributeMetaKind::List(arguments) => {
            let expressions = arguments
                .iter()
                .map(|argument| cfg_expression(argument.meta()?))
                .collect::<Option<Vec<_>>>()?;
            match (name.as_str(), expressions.len()) {
                ("all", _) => Some(CfgExpression::All { expressions, range }),
                ("any", _) => Some(CfgExpression::Any { expressions, range }),
                ("not", 1) => Some(CfgExpression::Not {
                    expression: Box::new(expressions.into_iter().next()?),
                    range,
                }),
                _ => None,
            }
        }
    }
}

fn string_value(meta: &AttributeMeta) -> Option<String> {
    let AttributeMetaKind::NameValue(value) = meta.kind() else {
        return None;
    };
    match value.kind() {
        AttributeMetaValueKind::Literal {
            kind: LiteralKind::String,
            decoded_string: Some(value),
            ..
        } => Some(value.clone()),
        _ => None,
    }
}

fn record_malformed(effects: &mut AttributeEffects, bounds: Range<usize>) {
    effects.malformed = true;
    if !effects.malformed_ranges.contains(&bounds) {
        effects.malformed_ranges.push(bounds);
    }
}

fn guarded_condition(guards: &[CfgCondition], condition: CfgCondition) -> CfgCondition {
    let Some(guard) = guard_condition(guards) else {
        return condition;
    };
    CfgCondition::any([guard.negate(), condition])
}

fn guard_condition(guards: &[CfgCondition]) -> Option<CfgCondition> {
    match guards {
        [] => None,
        [guard] => Some(guard.clone()),
        _ => Some(CfgCondition::all(guards.iter().cloned())),
    }
}
