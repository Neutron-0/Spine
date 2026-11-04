use crate::artifact::ArtifactService;
use crate::core::error::ForgeError;
use crate::core::types::*;
use crate::db::Database;
use std::sync::Arc;

pub struct PackageService {
    db: Arc<dyn Database>,
    artifacts: Arc<ArtifactService>,
}

impl PackageService {
    pub fn new(db: Arc<dyn Database>, artifacts: Arc<ArtifactService>) -> Self {
        Self { db, artifacts }
    }

    pub fn artifacts(&self) -> &Arc<ArtifactService> {
        &self.artifacts
    }

    pub fn db(&self) -> &Arc<dyn Database> {
        &self.db
    }

    pub async fn get_or_create_package(
        &self,
        ecosystem: &str,
        name: &str,
        owner_type: &str,
        owner_id: &str,
        description: &str,
    ) -> Result<Package, ForgeError> {
        if let Some(pkg) = self.db.get_package_by_name(ecosystem, name).await? {
            return Ok(pkg);
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as i64;

        let pkg = Package {
            id: uuid::Uuid::new_v4().to_string(),
            ecosystem: ecosystem.to_string(),
            name: name.to_string(),
            owner_type: owner_type.to_string(),
            owner_id: owner_id.to_string(),
            is_private: false,
            description: description.to_string(),
            created_at_nanos: now,
            updated_at_nanos: now,
        };

        self.db.create_package(pkg).await
    }

    pub async fn get_package(&self, ecosystem: &str, name: &str) -> Result<Option<PackageDto>, ForgeError> {
        let pkg = match self.db.get_package_by_name(ecosystem, name).await? {
            Some(p) => p,
            None => return Ok(None),
        };

        let versions = self.db.list_package_versions(&pkg.id).await?;
        Ok(Some(self.to_dto(pkg, versions)))
    }

    pub async fn list_packages(&self, ecosystem: &str) -> Result<Vec<PackageDto>, ForgeError> {
        let list = self.db.list_packages(ecosystem).await?;
        let mut dtos = Vec::with_capacity(list.len());
        for pkg in list {
            let versions = self.db.list_package_versions(&pkg.id).await?;
            dtos.push(self.to_dto(pkg, versions));
        }
        Ok(dtos)
    }

    pub async fn publish_version(
        &self,
        ecosystem: &str,
        name: &str,
        owner_type: &str,
        owner_id: &str,
        version: &str,
        manifest_id: &str,
        cksum: &str,
        size_bytes: i64,
        metadata_json: &str,
        user_id: &str,
    ) -> Result<PackageVersionDto, ForgeError> {
        let pkg = self
            .get_or_create_package(ecosystem, name, owner_type, owner_id, "")
            .await?;

        // Version immutability check
        if let Some(_) = self.db.get_package_version(&pkg.id, version).await? {
            return Err(ForgeError::Conflict(format!(
                "Version '{}' of package '{}' has already been published",
                version, name
            )));
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as i64;

        let ver = PackageVersion {
            id: uuid::Uuid::new_v4().to_string(),
            package_id: pkg.id,
            version: version.to_string(),
            manifest_id: manifest_id.to_string(),
            cksum: cksum.to_string(),
            size_bytes,
            metadata_json: metadata_json.to_string(),
            is_yanked: false,
            created_by_user_id: user_id.to_string(),
            created_at_nanos: now,
        };

        let created = self.db.create_package_version(ver).await?;
        Ok(self.version_to_dto(created))
    }

    pub async fn get_version(
        &self,
        ecosystem: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<PackageVersionDto>, ForgeError> {
        let pkg = match self.db.get_package_by_name(ecosystem, name).await? {
            Some(p) => p,
            None => return Ok(None),
        };

        let ver = self.db.get_package_version(&pkg.id, version).await?;
        Ok(ver.map(|v| self.version_to_dto(v)))
    }

    pub async fn set_yanked(
        &self,
        ecosystem: &str,
        name: &str,
        version: &str,
        yanked: bool,
    ) -> Result<(), ForgeError> {
        let pkg = self
            .db
            .get_package_by_name(ecosystem, name)
            .await?
            .ok_or_else(|| ForgeError::NotFound(format!("Package '{}' not found", name)))?;

        self.db
            .set_package_version_yanked(&pkg.id, version, yanked)
            .await
    }

    fn to_dto(&self, pkg: Package, versions: Vec<PackageVersion>) -> PackageDto {
        let count = versions.len();
        PackageDto {
            id: pkg.id,
            ecosystem: pkg.ecosystem,
            name: pkg.name,
            owner_type: pkg.owner_type,
            owner_id: pkg.owner_id,
            is_private: pkg.is_private,
            description: pkg.description,
            versions: versions.into_iter().map(|v| self.version_to_dto(v)).collect(),
            versions_count: count,
            created_at_nanos: pkg.created_at_nanos,
            updated_at_nanos: pkg.updated_at_nanos,
        }
    }

    fn version_to_dto(&self, v: PackageVersion) -> PackageVersionDto {
        PackageVersionDto {
            id: v.id,
            package_id: v.package_id,
            version: v.version,
            manifest_id: v.manifest_id,
            cksum: v.cksum,
            size_bytes: v.size_bytes,
            metadata_json: v.metadata_json,
            is_yanked: v.is_yanked,
            created_by_user_id: v.created_by_user_id,
            created_at_nanos: v.created_at_nanos,
        }
    }
}
