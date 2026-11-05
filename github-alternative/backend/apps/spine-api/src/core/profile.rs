use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Standard Deployment Profiles for Spine Forge.
///
/// Under `CoreForge` (`core-forge-v1`), untrusted CI execution is strictly disabled,
/// PostgreSQL persistence is required, and in-memory mock storage is forbidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum DeploymentProfile {
    /// Authoritative production profile for Core Forge.
    #[default]
    #[serde(rename = "core-forge-v1")]
    CoreForge,

    /// Local development profile.
    #[serde(rename = "development")]
    Development,

    /// Integration and regression test profile.
    #[serde(rename = "test")]
    Test,
}

impl DeploymentProfile {
    /// Return the canonical identifier string for this profile.
    pub fn name(&self) -> &'static str {
        match self {
            Self::CoreForge => "core-forge-v1",
            Self::Development => "development",
            Self::Test => "test",
        }
    }

    /// Whether this profile mandates a real, persistent PostgreSQL database.
    pub fn requires_persistent_database(&self) -> bool {
        match self {
            Self::CoreForge => true,
            Self::Development | Self::Test => false,
        }
    }

    /// Whether this profile permits running with an in-memory database.
    pub fn permits_in_memory_db(&self) -> bool {
        match self {
            Self::CoreForge => false,
            Self::Development | Self::Test => true,
        }
    }

    /// Whether untrusted workflow execution (external contributors, fork PRs, unverified runners)
    /// is permitted to execute on this instance.
    pub fn allow_untrusted_ci(&self) -> bool {
        match self {
            Self::CoreForge => false,
            Self::Development | Self::Test => true,
        }
    }

    /// Whether shared/public runners can be registered.
    pub fn allow_shared_runners(&self) -> bool {
        match self {
            Self::CoreForge => false,
            Self::Development | Self::Test => true,
        }
    }

    /// Parse a profile string with standard aliases.
    pub fn from_str_relaxed(s: &str) -> Result<Self, String> {
        let normalized = s.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "core-forge-v1" | "core-forge" | "production" | "prod" => Ok(Self::CoreForge),
            "development" | "dev" => Ok(Self::Development),
            "test" | "testing" => Ok(Self::Test),
            unknown => Err(format!(
                "Unknown deployment profile '{}'. Supported profiles: core-forge-v1, development, test",
                unknown
            )),
        }
    }
}

impl FromStr for DeploymentProfile {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str_relaxed(s)
    }
}

impl fmt::Display for DeploymentProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}
