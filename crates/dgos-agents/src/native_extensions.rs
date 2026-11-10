//! Host-only native configuration operations. No type in this module accepts a
//! renderer-supplied absolute path, and no operation executes a CLI or helper.
//! Durable publication is deliberately macOS-only; other platforms fail closed.
mod model;
#[cfg(unix)]
mod paths;
mod config;
mod operations;
pub mod mcp;
pub mod skills;
pub mod package;
pub mod dependencies;
#[cfg(unix)]
mod resource_tree;
mod resources;
pub use resources::{ResourceAction, ResourceDiscovery, ResourceImpact, ResourceInstance, ResourceLink, ResourceLinkStatus, ResourceNameMatch, ResourcePreview, ResourceSource, ResourceSummary};
#[cfg(all(test, target_os = "macos"))]
mod resource_tests;

pub use model::{Access, Error, HostContext, OperationId, Owner, Receipt, Scope, State, Target, TrustedRoots};
pub use operations::{NativeExtensions, Preview};

/// Host-only recovery location. Intentionally neither Serialize nor Debug:
/// callers may present it only through an explicit recovery workflow.
pub struct RecoveryMaterial { pub directory: std::path::PathBuf }

#[cfg(all(test, target_os = "macos"))]
mod tests;
