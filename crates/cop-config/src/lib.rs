#![forbid(unsafe_code)]

mod load;
mod resolve;
mod schema;

pub use load::{ConfigError, ConfigLoader, ConfigRequest, LoadedConfig};
pub use resolve::{
    ConfigOverrides, ConfigResolutionInput, ConfigResolver, ResolvedConfig, RuleSetting,
};
pub use schema::{ScanDefaults, ScanTargetScope};
