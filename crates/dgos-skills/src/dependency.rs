use crate::entity::{DependencyConfig, DependencyStatus};
use crate::error::{Result, SkillError};
use semver::Version;
use std::process::Command;

pub struct DependencyChecker;

impl DependencyChecker {
    pub fn check_dependencies(deps: &DependencyConfig) -> Result<DependencyStatus> {
        let mut missing = Vec::new();

        // Check Python
        if let Some(python_req) = &deps.python {
            match Self::check_python_version(python_req) {
                Ok(false) => missing.push(format!("Python {}", python_req)),
                Err(_) => missing.push(format!("Python {} (not installed)", python_req)),
                _ => {}
            }
        }

        // Check Node.js
        if let Some(node_req) = &deps.node {
            match Self::check_node_version(node_req) {
                Ok(false) => missing.push(format!("Node.js {}", node_req)),
                Err(_) => missing.push(format!("Node.js {} (not installed)", node_req)),
                _ => {}
            }
        }

        Ok(DependencyStatus {
            satisfied: missing.is_empty(),
            missing,
        })
    }

    fn check_python_version(requirement: &str) -> Result<bool> {
        let output = Command::new("python3")
            .arg("--version")
            .output()
            .map_err(|_| SkillError::DependencyNotSatisfied("Python not found".to_string()))?;

        let version_str = String::from_utf8_lossy(&output.stdout);
        let version = version_str.trim().strip_prefix("Python ").ok_or_else(|| {
            SkillError::DependencyNotSatisfied("Invalid Python version".to_string())
        })?;

        Self::version_satisfies(version, requirement)
    }

    fn check_node_version(requirement: &str) -> Result<bool> {
        let output = Command::new("node")
            .arg("--version")
            .output()
            .map_err(|_| SkillError::DependencyNotSatisfied("Node.js not found".to_string()))?;

        let version_str = String::from_utf8_lossy(&output.stdout);
        let version = version_str
            .trim()
            .strip_prefix('v')
            .unwrap_or(version_str.trim());

        Self::version_satisfies(version, requirement)
    }

    fn version_satisfies(version: &str, requirement: &str) -> Result<bool> {
        // Parse version
        let parts: Vec<&str> = version.split('.').collect();
        if parts.len() < 2 {
            return Err(SkillError::Internal("Invalid version format".to_string()));
        }

        let version_str = if parts.len() == 2 {
            format!("{}.0", version)
        } else {
            version.to_string()
        };

        let ver = Version::parse(&version_str)
            .map_err(|_| SkillError::Internal("Failed to parse version".to_string()))?;

        fn normalize_requirement(input: &str) -> String {
            let (operator, raw) = if let Some(value) = input.strip_prefix(">=") {
                (">=", value.trim())
            } else {
                ("", input.trim())
            };
            let mut parts = raw.split('.').collect::<Vec<_>>();
            while parts.len() < 3 {
                parts.push("0");
            }
            format!("{}{}", operator, parts.join("."))
        }

        // Parse requirement (simple >= support), accepting Python/Node style
        // two-part versions such as `>=3.10`.
        let normalized = normalize_requirement(requirement);
        if let Some(req_ver_str) = normalized.strip_prefix(">=") {
            let req_ver = Version::parse(req_ver_str.trim())
                .map_err(|_| SkillError::Internal("Failed to parse requirement".to_string()))?;
            Ok(ver >= req_ver)
        } else {
            // Exact match
            let req_ver = Version::parse(normalized.trim())
                .map_err(|_| SkillError::Internal("Failed to parse requirement".to_string()))?;
            Ok(ver == req_ver)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_satisfies() {
        assert!(DependencyChecker::version_satisfies("3.10.5", ">=3.10").unwrap());
        assert!(DependencyChecker::version_satisfies("3.11.0", ">=3.10").unwrap());
        assert!(!DependencyChecker::version_satisfies("3.9.0", ">=3.10").unwrap());
    }
}
