#![forbid(unsafe_code)]

mod dependency_index;
mod feature_resolver;
mod metadata;
mod selection;

pub use cop_cfg::CfgValue;
pub use dependency_index::{CargoDependency, DependencyIndex, DependencyKind};
pub use feature_resolver::{FeatureSelection, ResolvedFeatures};
pub use metadata::{
    CargoDiscovery, CargoDiscoveryRequest, CargoError, CargoPackage, CargoProjectLoader,
    CargoTarget,
};
pub use selection::{CargoProject, CargoSelection, SelectedPackage, TargetContext, TargetSelector};
