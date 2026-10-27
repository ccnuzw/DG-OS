use thiserror::Error;

#[derive(Error, Debug)]
pub enum SkillError {
    #[error("Skill not found: {0}")]
    SkillNotFound(String),

    #[error("Skill already installed: {0}")]
    SkillAlreadyInstalled(String),

    #[error("Dependency not satisfied: {0}")]
    DependencyNotSatisfied(String),

    #[error("Skill is disabled: {0}")]
    SkillDisabled(String),

    #[error("Skill not configured: {0}")]
    SkillNotConfigured(String),

    #[error("HTTP skill not started: {0}")]
    SkillNotStarted(String),

    #[error("Skill execution failed: {0}")]
    SkillExecutionFailed(String),

    #[error("Skill timeout: {0}")]
    SkillTimeout(String),

    #[error("HTTP skill start timeout: {0}")]
    SkillStartTimeout(String),

    #[error("Registry unreachable: {0}")]
    RegistryUnreachable(String),

    #[error("Invalid manifest: {0}")]
    InvalidManifest(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Not an HTTP skill")]
    NotHttpSkill,

    #[error("Not a CLI skill")]
    NotCliSkill,

    #[error("Content skill is prepared as context, not executed")]
    ContentSkillNotExecutable,

    #[error("Legacy HTTP/CLI packages are read-only; installation, execution and removal require migration")]
    LegacyFormatUnsupported,

    #[error("Internal error: {0}")]
    Internal(String),
}

impl SkillError {
    pub fn error_code(&self) -> &str {
        match self {
            SkillError::SkillNotFound(_) => "1002001",
            SkillError::SkillAlreadyInstalled(_) => "1002002",
            SkillError::DependencyNotSatisfied(_) => "1002003",
            SkillError::SkillDisabled(_) => "1002004",
            SkillError::SkillNotConfigured(_) => "1002005",
            SkillError::SkillNotStarted(_) => "1002006",
            SkillError::SkillExecutionFailed(_) => "1002007",
            SkillError::SkillStartTimeout(_) => "1002009",
            SkillError::RegistryUnreachable(_) => "1002010",
            _ => "1002999",
        }
    }
}

pub type Result<T, E = SkillError> = std::result::Result<T, E>;
