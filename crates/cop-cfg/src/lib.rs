#![forbid(unsafe_code)]

mod evaluator;
mod expression;

pub use evaluator::{CfgContext, CfgDecision, CfgEvaluator, CfgValue, ScopedCfgValue};
pub use expression::{CfgCondition, CfgExpression, CfgParseError};

pub fn is_builtin_cfg_name(name: &str) -> bool {
    CfgValue::is_builtin_name(name)
}
