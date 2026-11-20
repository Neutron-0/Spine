use crate::core::error::ForgeError;
use crate::packages::service::PackageService;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;

pub const CARGO_ECOSYSTEM: &str = "cargo";

#[derive(Debug, Serialize, Deserialize)]
pub struct CargoPublishMetadata {
    pub name: String,
    pub vers: String,
    #[serde(default)]
    pub deps: Vec<CargoPublishDependency>,
    #[serde(default)]
    pub features: HashMap<String, Vec<String>>,
    pub authors: Option<Vec<String>>,
    pub description: Option<String>,
    pub documentation: Option<String>,
    pub homepage: Option<String>,
    pub readme: Option<String>,
    pub keywords: Option<Vec<String>>,
    pub categories: Option<Vec<String>>,
    pub license: Option<String>,
    pub repository: Option<String>,
    pub links: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CargoPublishDependency {
    pub name: String,
    pub version_req: String,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub optional: bool,
    #[serde(default = "default_true")]
    pub default_features: bool,
    pub target: Option<String>,
    pub kind: Option<String>,
    pub registry: Option<String>,
    pub explicit_name_in_toml: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CargoIndexEntry {
    pub name: String,
    pub vers: String,
    pub deps: Vec<CargoIndexDependency>,
    pub cksum: String,
    pub features: HashMap<String, Vec<String>>,
    pub yanked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<String>,
    pub v: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CargoIndexDependency {
    pub name: String,
    pub req: String,
    pub features: Vec<String>,
    pub optional: bool,
    pub default_features: bool,
    pub target: Option<String>,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CargoPublishResponse {
    pub warnings: CargoPublishWarnings,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CargoPublishWarnings {
    pub invalid_categories: Vec<String>,
    pub invalid_badges: Vec<String>,
}

pub struct CargoRegistryAdapter {
    package_service: Arc<PackageService>,
}

impl CargoRegistryAdapter {
    pub fn new(package_service: Arc<PackageService>) -> Self {
        Self { package_service }
    }

    pub fn crate_prefix(name: &str) -> String {
        let name = name.to_lowercase();
        match name.len() {
            0 => String::new(),
            1 => "1".to_string(),
            2 => "2".to_string(),
            3 => format!("3/{}", &name[0..1]),
            _ => format!("{}/{}", &name[0..2], &name[2..4]),
        }
    }

    pub fn config_json(base_url: &str) -> serde_json::Value {
        serde_json::json!({
            "dl": format!("{}/api/v1/crates/{{crate}}/{{version}}/download", base_url.trim_end_matches('/')),
            "api": base_url.trim_end_matches('/')
        })
    }

    /// Parses the binary payload sent by `cargo publish`
    pub fn parse_publish_payload(
        bytes: &[u8],
    ) -> Result<(CargoPublishMetadata, Vec<u8>, String), ForgeError> {
        if bytes.len() < 8 {
            return Err(ForgeError::BadRequest(
                "Payload too short for cargo publish".to_string(),
            ));
        }

        let json_len = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let json_start = 4;
        let json_end = json_start + json_len;

        if bytes.len() < json_end + 4 {
            return Err(ForgeError::BadRequest(
                "Malformed cargo publish header: unexpected EOF".to_string(),
            ));
        }

        let meta_json_slice = &bytes[json_start..json_end];
        let meta: CargoPublishMetadata = serde_json::from_slice(meta_json_slice).map_err(|e| {
            ForgeError::BadRequest(format!("Failed to parse crate metadata JSON: {}", e))
        })?;

        let crate_len = u32::from_le_bytes([
            bytes[json_end],
            bytes[json_end + 1],
            bytes[json_end + 2],
            bytes[json_end + 3],
        ]) as usize;
        let crate_start = json_end + 4;
        let crate_end = crate_start + crate_len;

        if bytes.len() < crate_end {
            return Err(ForgeError::BadRequest(
                "Malformed crate payload: tarball bytes truncated".to_string(),
            ));
        }

        let crate_bytes = bytes[crate_start..crate_end].to_vec();

        // Calculate SHA-256 checksum for the Cargo index
        let mut hasher = Sha256::new();
        hasher.update(&crate_bytes);
        let cksum = hex::encode(hasher.finalize());

        Ok((meta, crate_bytes, cksum))
    }

    /// Publishes a new crate version from raw `cargo publish` binary payload
    pub async fn publish(
        &self,
        payload: &[u8],
        user_id: &str,
    ) -> Result<CargoPublishResponse, ForgeError> {
        let (meta, crate_bytes, cksum) = Self::parse_publish_payload(payload)?;

        let artifact_name = format!("{}-{}.crate", meta.name, meta.vers);
        let stored_manifest = self
            .package_service
            .artifacts()
            .store_object_bytes(
                "user",
                user_id,
                &artifact_name,
                Some("application/x-tar".to_string()),
                &crate_bytes,
            )
            .await?;

        // Construct Cargo index entry
        let index_entry = CargoIndexEntry {
            name: meta.name.clone(),
            vers: meta.vers.clone(),
            deps: meta
                .deps
                .into_iter()
                .map(|d| CargoIndexDependency {
                    name: d.explicit_name_in_toml.clone().unwrap_or(d.name.clone()),
                    req: d.version_req,
                    features: d.features,
                    optional: d.optional,
                    default_features: d.default_features,
                    target: d.target,
                    kind: d.kind.unwrap_or_else(|| "normal".to_string()),
                    registry: d.registry,
                    package: if d.explicit_name_in_toml.is_some() {
                        Some(d.name)
                    } else {
                        None
                    },
                })
                .collect(),
            cksum: cksum.clone(),
            features: meta.features,
            yanked: false,
            links: meta.links,
            v: 2,
        };

        let metadata_json = serde_json::to_string(&index_entry)
            .map_err(|e| ForgeError::Internal(format!("Failed to serialize index entry: {}", e)))?;

        self.package_service
            .publish_version(
                CARGO_ECOSYSTEM,
                &meta.name,
                "user",
                user_id,
                &meta.vers,
                &stored_manifest.id,
                &cksum,
                crate_bytes.len() as i64,
                &metadata_json,
                user_id,
            )
            .await?;

        Ok(CargoPublishResponse {
            warnings: CargoPublishWarnings {
                invalid_categories: vec![],
                invalid_badges: vec![],
            },
        })
    }

    /// Returns the sparse index newline-delimited JSON for a crate
    pub async fn get_index_file(&self, crate_name: &str) -> Result<Option<String>, ForgeError> {
        let pkg = self
            .package_service
            .get_package(CARGO_ECOSYSTEM, crate_name)
            .await?;

        let pkg = match pkg {
            Some(p) => p,
            None => return Ok(None),
        };

        if pkg.versions.is_empty() {
            return Ok(None);
        }

        let mut lines = Vec::new();
        for v in pkg.versions {
            // Check if yanked status changed
            let mut entry: serde_json::Value = serde_json::from_str(&v.metadata_json)
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
            entry["yanked"] = serde_json::json!(v.is_yanked);
            lines.push(
                serde_json::to_string(&entry).map_err(|e| ForgeError::Internal(e.to_string()))?,
            );
        }

        let index_content = lines.join("\n") + "\n";
        Ok(Some(index_content))
    }

    /// Downloads the `.crate` tarball for a given package and version
    pub async fn download(&self, crate_name: &str, version: &str) -> Result<Vec<u8>, ForgeError> {
        let ver = self
            .package_service
            .get_version(CARGO_ECOSYSTEM, crate_name, version)
            .await?
            .ok_or_else(|| {
                ForgeError::NotFound(format!(
                    "Crate {} version {} not found",
                    crate_name, version
                ))
            })?;

        self.package_service
            .artifacts()
            .reconstruct_object(&ver.manifest_id)
            .await
    }

    /// Yanks or unyanks a crate version
    pub async fn set_yanked(
        &self,
        crate_name: &str,
        version: &str,
        yanked: bool,
        user_id: &str,
    ) -> Result<(), ForgeError> {
        self.package_service
            .set_yanked(CARGO_ECOSYSTEM, crate_name, version, yanked, user_id)
            .await
    }
}
