#![forbid(unsafe_code)]

mod evaluator;
mod expression;

pub use evaluator::{CfgContext, CfgDecision, CfgEvaluator, CfgValue};
pub use expression::{CfgCondition, CfgExpression, CfgParseError};
