mod content_manager;
pub mod content_package;
pub mod dependency;
pub mod entity;
pub mod error;
pub mod github;
pub mod install;
pub mod manager;
pub mod package_validation;
pub mod repository;
pub mod source_parser;

#[cfg(test)]
mod registry_tests;

pub use dependency::DependencyChecker;
pub use entity::*;
pub use error::{Result, SkillError};
pub use github::{GithubAdapter, ResolvedGithubSource};
pub use install::{InstallCoordinator, InstalledSkill, SkillInstallPreview};
pub use manager::SkillsManager;
pub(crate) use package_validation::content_hash;
pub use package_validation::{
    validate_external_package, validate_external_zip, validate_external_zip_at_subpath,
    ExternalSkillMetadata, LicenseStatus, ValidatedExternalPackage, MAX_EXTERNAL_ARCHIVE_BYTES,
};
pub use repository::{RegistryRepository, SkillRepository};
pub use source_parser::{parse_command, parse_source, GithubSource, SkillSource};
