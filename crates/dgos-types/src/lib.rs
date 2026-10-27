// DG-OS Common Types
// Shared types across all modules

pub mod error;
pub mod provider;

pub use error::{DgosError, DgosResult};
pub use provider::*;
