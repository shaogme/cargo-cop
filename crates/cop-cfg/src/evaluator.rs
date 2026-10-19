use std::collections::BTreeSet;

use crate::{CfgCondition, CfgExpression};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CfgValue {
    name: String,
    value: Option<String>,
}

pub struct CfgContext<'a> {
    values: &'a [CfgValue],
    features: Option<&'a BTreeSet<String>>,
    test: Option<bool>,
    proc_macro: Option<bool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CfgDecision {
    True,
    False,
    Unknown,
}

pub struct CfgEvaluator<'context, 'values> {
    context: &'context CfgContext<'values>,
}

impl CfgValue {
    pub fn new(name: impl Into<String>, value: Option<String>) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }
}

impl<'a> CfgContext<'a> {
    pub fn new(values: &'a [CfgValue], features: Option<&'a BTreeSet<String>>) -> Self {
        Self {
            values,
            features,
            test: None,
            proc_macro: None,
        }
    }

    pub fn with_test(mut self, test: Option<bool>) -> Self {
        self.test = test;
        self
    }

    pub fn with_proc_macro(mut self, proc_macro: Option<bool>) -> Self {
        self.proc_macro = proc_macro;
        self
    }

    fn values(&self) -> &[CfgValue] {
        self.values
    }

    fn features(&self) -> Option<&BTreeSet<String>> {
        self.features
    }

    fn test(&self) -> Option<bool> {
        self.test
    }

    fn proc_macro(&self) -> Option<bool> {
        self.proc_macro
    }
}

impl CfgDecision {
    pub fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::False, _) | (_, Self::False) => Self::False,
            (Self::True, Self::True) => Self::True,
            _ => Self::Unknown,
        }
    }

    pub fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::True, _) | (_, Self::True) => Self::True,
            (Self::False, Self::False) => Self::False,
            _ => Self::Unknown,
        }
    }

    pub fn negate(self) -> Self {
        match self {
            Self::True => Self::False,
            Self::False => Self::True,
            Self::Unknown => Self::Unknown,
        }
    }
}

impl<'context, 'values> CfgEvaluator<'context, 'values> {
    pub fn new(context: &'context CfgContext<'values>) -> Self {
        Self { context }
    }

    pub fn evaluate(&self, expression: &CfgExpression) -> CfgDecision {
        match expression {
            CfgExpression::Flag { name, .. } => self.evaluate_atom(name, None),
            CfgExpression::KeyValue { name, value, .. } => self.evaluate_atom(name, Some(value)),
            CfgExpression::All { expressions, .. } => expressions
                .iter()
                .map(|expression| self.evaluate(expression))
                .fold(CfgDecision::True, CfgDecision::and),
            CfgExpression::Any { expressions, .. } => expressions
                .iter()
                .map(|expression| self.evaluate(expression))
                .fold(CfgDecision::False, CfgDecision::or),
            CfgExpression::Not { expression, .. } => self.evaluate(expression).negate(),
        }
    }

    pub fn evaluate_condition(&self, condition: &CfgCondition) -> CfgDecision {
        condition.evaluate(&|expression| self.evaluate(expression))
    }

    fn evaluate_atom(&self, name: &str, expected: Option<&str>) -> CfgDecision {
        if name == "feature" {
            return match (expected, self.context.features()) {
                (Some(feature), Some(features)) => {
                    if features.contains(feature) {
                        CfgDecision::True
                    } else {
                        CfgDecision::False
                    }
                }
                _ => CfgDecision::Unknown,
            };
        }
        if name == "test" {
            return self.context_boolean(self.context.test(), expected);
        }
        if name == "proc_macro" {
            return self.context_boolean(self.context.proc_macro(), expected);
        }
        if !is_known_cfg_name(name)
            && !self
                .context
                .values()
                .iter()
                .any(|value| value.name() == name)
        {
            return CfgDecision::Unknown;
        }
        if self
            .context
            .values()
            .iter()
            .any(|value| value.name() == name && value.value() == expected)
        {
            CfgDecision::True
        } else {
            CfgDecision::False
        }
    }

    fn context_boolean(&self, actual: Option<bool>, expected: Option<&str>) -> CfgDecision {
        if expected.is_some() {
            return CfgDecision::False;
        }
        match actual {
            Some(true) => CfgDecision::True,
            Some(false) => CfgDecision::False,
            None => CfgDecision::Unknown,
        }
    }
}

fn is_known_cfg_name(name: &str) -> bool {
    matches!(
        name,
        "debug_assertions"
            | "panic"
            | "target_abi"
            | "target_arch"
            | "target_endian"
            | "target_env"
            | "target_family"
            | "target_feature"
            | "target_has_atomic"
            | "target_has_atomic_equal_alignment"
            | "target_has_atomic_load_store"
            | "target_has_atomic_primitive_alignment"
            | "target_os"
            | "target_pointer_width"
            | "target_thread_local"
            | "target_vendor"
            | "unix"
            | "windows"
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{CfgContext, CfgDecision, CfgEvaluator, CfgExpression, CfgValue};

    #[test]
    fn evaluates_target_feature_test_and_proc_macro_contexts() {
        let values = [
            CfgValue::new("unix", None),
            CfgValue::new("target_os", Some("linux".to_owned())),
        ];
        let features = BTreeSet::from(["enabled".to_owned()]);
        let context = CfgContext::new(&values, Some(&features))
            .with_test(Some(true))
            .with_proc_macro(Some(false));
        let evaluator = CfgEvaluator::new(&context);

        assert_eq!(
            evaluator.evaluate(&CfgExpression::parse("target_os = \"linux\"").expect("cfg")),
            CfgDecision::True
        );
        assert_eq!(
            evaluator.evaluate(&CfgExpression::parse("target_vendor = \"none\"").expect("cfg")),
            CfgDecision::False
        );
        assert_eq!(
            evaluator.evaluate(&CfgExpression::parse("feature = \"enabled\"").expect("cfg")),
            CfgDecision::True
        );
        assert_eq!(
            evaluator.evaluate(&CfgExpression::parse("test").expect("cfg")),
            CfgDecision::True
        );
        assert_eq!(
            evaluator.evaluate(&CfgExpression::parse("proc_macro").expect("cfg")),
            CfgDecision::False
        );
    }

    #[test]
    fn preserves_unknown_custom_cfgs_through_boolean_operators() {
        let context = CfgContext::new(&[], None);
        let evaluator = CfgEvaluator::new(&context);

        assert_eq!(
            evaluator.evaluate(&CfgExpression::parse("custom_mode").expect("cfg")),
            CfgDecision::Unknown
        );
        assert_eq!(
            evaluator.evaluate(&CfgExpression::parse("not(custom_mode)").expect("cfg")),
            CfgDecision::Unknown
        );
        assert_eq!(
            evaluator.evaluate(&CfgExpression::parse("all(custom_mode, any())").expect("cfg")),
            CfgDecision::False
        );
        assert_eq!(
            evaluator.evaluate(&CfgExpression::parse("any(custom_mode, all())").expect("cfg")),
            CfgDecision::True
        );
    }
}
