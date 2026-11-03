use super::traits::Database;
use crate::core::error::ForgeError;
use crate::core::types::*;
use async_trait::async_trait;
use sqlx::{PgPool, Row};

pub struct PgDatabase {
    pool: PgPool,
}

impl PgDatabase {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn connect(url: &str) -> Result<Self, ForgeError> {
        let pool = PgPool::connect(url)
            .await
            .map_err(|e| ForgeError::ServiceUnavailable(format!("PostgreSQL connect failed: {}", e)))?;
        Ok(Self { pool })
    }
}

#[async_trait]
impl Database for PgDatabase {
    async fn create_user(&self, user: User) -> Result<User, ForgeError> {
        let query = r#"
            INSERT INTO users (id, username, normalized_username, email, normalized_email, display_name, password_hash, account_status, avatar_url, created_at_nanos, updated_at_nanos, last_login_nanos)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            ON CONFLICT (normalized_username) DO NOTHING
        "#;
        let res = sqlx::query(query)
            .bind(&user.id)
            .bind(&user.username)
            .bind(&user.normalized_username)
            .bind(&user.email)
            .bind(&user.normalized_email)
            .bind(&user.display_name)
            .bind(&user.password_hash)
            .bind(&user.account_status)
            .bind(&user.avatar_url)
            .bind(user.created_at_nanos)
            .bind(user.updated_at_nanos)
            .bind(user.last_login_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if res.rows_affected() == 0 {
            return Err(ForgeError::Conflict(format!("Username or email is already taken")));
        }
        Ok(user)
    }

    async fn get_user_by_id(&self, id: &str) -> Result<Option<User>, ForgeError> {
        let query = "SELECT id, username, normalized_username, email, normalized_email, display_name, password_hash, account_status, avatar_url, created_at_nanos, updated_at_nanos, last_login_nanos FROM users WHERE id = $1";
        let row = sqlx::query(query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            Ok(Some(User {
                id: r.get(0),
                username: r.get(1),
                normalized_username: r.get(2),
                email: r.get(3),
                normalized_email: r.get(4),
                display_name: r.get(5),
                password_hash: r.get(6),
                account_status: r.get(7),
                avatar_url: r.get(8),
                created_at_nanos: r.get(9),
                updated_at_nanos: r.get(10),
                last_login_nanos: r.get(11),
            }))
        } else {
            Ok(None)
        }
    }

    async fn get_user_by_username(&self, username: &str) -> Result<Option<User>, ForgeError> {
        let norm = username.trim().to_lowercase();
        let query = "SELECT id, username, normalized_username, email, normalized_email, display_name, password_hash, account_status, avatar_url, created_at_nanos, updated_at_nanos, last_login_nanos FROM users WHERE normalized_username = $1";
        let row = sqlx::query(query)
            .bind(&norm)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            Ok(Some(User {
                id: r.get(0),
                username: r.get(1),
                normalized_username: r.get(2),
                email: r.get(3),
                normalized_email: r.get(4),
                display_name: r.get(5),
                password_hash: r.get(6),
                account_status: r.get(7),
                avatar_url: r.get(8),
                created_at_nanos: r.get(9),
                updated_at_nanos: r.get(10),
                last_login_nanos: r.get(11),
            }))
        } else {
            Ok(None)
        }
    }

    async fn get_user_by_email(&self, email: &str) -> Result<Option<User>, ForgeError> {
        let norm = email.trim().to_lowercase();
        let query = "SELECT id, username, normalized_username, email, normalized_email, display_name, password_hash, account_status, avatar_url, created_at_nanos, updated_at_nanos, last_login_nanos FROM users WHERE normalized_email = $1";
        let row = sqlx::query(query)
            .bind(&norm)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            Ok(Some(User {
                id: r.get(0),
                username: r.get(1),
                normalized_username: r.get(2),
                email: r.get(3),
                normalized_email: r.get(4),
                display_name: r.get(5),
                password_hash: r.get(6),
                account_status: r.get(7),
                avatar_url: r.get(8),
                created_at_nanos: r.get(9),
                updated_at_nanos: r.get(10),
                last_login_nanos: r.get(11),
            }))
        } else {
            Ok(None)
        }
    }

    async fn update_user_last_login(&self, user_id: &str, now_nanos: i64) -> Result<(), ForgeError> {
        let query = "UPDATE users SET last_login_nanos = $1, updated_at_nanos = $1 WHERE id = $2";
        sqlx::query(query)
            .bind(now_nanos)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn update_user_password(
        &self,
        user_id: &str,
        new_hash: &str,
        now_nanos: i64,
    ) -> Result<(), ForgeError> {
        let query = "UPDATE users SET password_hash = $1, updated_at_nanos = $2 WHERE id = $3";
        let res = sqlx::query(query)
            .bind(new_hash)
            .bind(now_nanos)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if res.rows_affected() == 0 {
            return Err(ForgeError::NotFound("User not found".into()));
        }
        Ok(())
    }

    async fn create_session(&self, session: UserSession) -> Result<UserSession, ForgeError> {
        let query = r#"
            INSERT INTO user_sessions (id, user_id, session_token_hash, ip_address, user_agent, created_at_nanos, expires_at_nanos, revoked)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#;
        sqlx::query(query)
            .bind(&session.id)
            .bind(&session.user_id)
            .bind(&session.session_token_hash)
            .bind(&session.ip_address)
            .bind(&session.user_agent)
            .bind(session.created_at_nanos)
            .bind(session.expires_at_nanos)
            .bind(session.revoked)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(session)
    }

    async fn get_session_by_hash(&self, token_hash: &str) -> Result<Option<UserSession>, ForgeError> {
        let query = "SELECT id, user_id, session_token_hash, ip_address, user_agent, created_at_nanos, expires_at_nanos, revoked FROM user_sessions WHERE session_token_hash = $1";
        let row = sqlx::query(query)
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            Ok(Some(UserSession {
                id: r.get(0),
                user_id: r.get(1),
                session_token_hash: r.get(2),
                ip_address: r.get(3),
                user_agent: r.get(4),
                created_at_nanos: r.get(5),
                expires_at_nanos: r.get(6),
                revoked: r.get(7),
            }))
        } else {
            Ok(None)
        }
    }

    async fn revoke_session(&self, session_id: &str) -> Result<(), ForgeError> {
        let query = "UPDATE user_sessions SET revoked = TRUE WHERE id = $1";
        sqlx::query(query)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<(), ForgeError> {
        let query = "UPDATE user_sessions SET revoked = TRUE WHERE user_id = $1";
        sqlx::query(query)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_user_sessions(&self, user_id: &str) -> Result<Vec<UserSession>, ForgeError> {
        let query = "SELECT id, user_id, session_token_hash, ip_address, user_agent, created_at_nanos, expires_at_nanos, revoked FROM user_sessions WHERE user_id = $1 AND revoked = FALSE ORDER BY created_at_nanos DESC";
        let rows = sqlx::query(query)
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(UserSession {
                id: r.get(0),
                user_id: r.get(1),
                session_token_hash: r.get(2),
                ip_address: r.get(3),
                user_agent: r.get(4),
                created_at_nanos: r.get(5),
                expires_at_nanos: r.get(6),
                revoked: r.get(7),
            });
        }
        Ok(list)
    }

    async fn create_email_token(
        &self,
        token_hash: &str,
        user_id: &str,
        expires_at_nanos: i64,
    ) -> Result<(), ForgeError> {
        let query = "INSERT INTO email_verification_tokens (token_hash, user_id, expires_at_nanos) VALUES ($1, $2, $3)";
        sqlx::query(query)
            .bind(token_hash)
            .bind(user_id)
            .bind(expires_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn consume_email_token(
        &self,
        token_hash: &str,
        now_nanos: i64,
    ) -> Result<Option<String>, ForgeError> {
        let query = "UPDATE email_verification_tokens SET used_at_nanos = $1 WHERE token_hash = $2 AND used_at_nanos IS NULL AND expires_at_nanos >= $1 RETURNING user_id";
        let row = sqlx::query(query)
            .bind(now_nanos)
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(row.map(|r| r.get(0)))
    }

    async fn create_reset_token(
        &self,
        token_hash: &str,
        user_id: &str,
        expires_at_nanos: i64,
    ) -> Result<(), ForgeError> {
        let query = "INSERT INTO password_reset_tokens (token_hash, user_id, expires_at_nanos) VALUES ($1, $2, $3)";
        sqlx::query(query)
            .bind(token_hash)
            .bind(user_id)
            .bind(expires_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn consume_reset_token(
        &self,
        token_hash: &str,
        now_nanos: i64,
    ) -> Result<Option<String>, ForgeError> {
        let query = "UPDATE password_reset_tokens SET used_at_nanos = $1 WHERE token_hash = $2 AND used_at_nanos IS NULL AND expires_at_nanos >= $1 RETURNING user_id";
        let row = sqlx::query(query)
            .bind(now_nanos)
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(row.map(|r| r.get(0)))
    }

    async fn create_pat(&self, pat: PersonalAccessToken) -> Result<PersonalAccessToken, ForgeError> {
        let query = r#"
            INSERT INTO personal_access_tokens (id, user_id, name, token_hash, scopes, expires_at_nanos, created_at_nanos, revoked)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#;
        let scopes_json = serde_json::to_value(&pat.scopes).unwrap_or(serde_json::json!([]));
        sqlx::query(query)
            .bind(&pat.id)
            .bind(&pat.user_id)
            .bind(&pat.name)
            .bind(&pat.token_hash)
            .bind(scopes_json)
            .bind(pat.expires_at_nanos)
            .bind(pat.created_at_nanos)
            .bind(pat.revoked)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(pat)
    }

    async fn get_pat_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<PersonalAccessToken>, ForgeError> {
        let query = "SELECT id, user_id, name, token_hash, scopes, expires_at_nanos, created_at_nanos, last_used_at_nanos, revoked FROM personal_access_tokens WHERE token_hash = $1";
        let row = sqlx::query(query)
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            let scopes_val: serde_json::Value = r.get(4);
            let scopes: Vec<String> = serde_json::from_value(scopes_val).unwrap_or_default();
            Ok(Some(PersonalAccessToken {
                id: r.get(0),
                user_id: r.get(1),
                name: r.get(2),
                token_hash: r.get(3),
                scopes,
                expires_at_nanos: r.get(5),
                created_at_nanos: r.get(6),
                last_used_at_nanos: r.get(7),
                revoked: r.get(8),
            }))
        } else {
            Ok(None)
        }
    }

    async fn update_pat_last_used(&self, pat_id: &str, now_nanos: i64) -> Result<(), ForgeError> {
        let query = "UPDATE personal_access_tokens SET last_used_at_nanos = $1 WHERE id = $2";
        sqlx::query(query)
            .bind(now_nanos)
            .bind(pat_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_user_pats(&self, user_id: &str) -> Result<Vec<PersonalAccessToken>, ForgeError> {
        let query = "SELECT id, user_id, name, token_hash, scopes, expires_at_nanos, created_at_nanos, last_used_at_nanos, revoked FROM personal_access_tokens WHERE user_id = $1 AND revoked = FALSE ORDER BY created_at_nanos DESC";
        let rows = sqlx::query(query)
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            let scopes_val: serde_json::Value = r.get(4);
            let scopes: Vec<String> = serde_json::from_value(scopes_val).unwrap_or_default();
            list.push(PersonalAccessToken {
                id: r.get(0),
                user_id: r.get(1),
                name: r.get(2),
                token_hash: r.get(3),
                scopes,
                expires_at_nanos: r.get(5),
                created_at_nanos: r.get(6),
                last_used_at_nanos: r.get(7),
                revoked: r.get(8),
            });
        }
        Ok(list)
    }

    async fn revoke_pat(&self, pat_id: &str, user_id: &str) -> Result<(), ForgeError> {
        let query = "UPDATE personal_access_tokens SET revoked = TRUE WHERE id = $1 AND user_id = $2";
        let res = sqlx::query(query)
            .bind(pat_id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if res.rows_affected() == 0 {
            return Err(ForgeError::NotFound("Personal access token not found".into()));
        }
        Ok(())
    }

    async fn create_repository(&self, repo: ForgeRepository) -> Result<ForgeRepository, ForgeError> {
        let query = r#"
            INSERT INTO forge_repositories (id, owner_user_id, owner_username, name, normalized_name, description, is_private, default_branch, native_repo_id, durability_policy, provisioning_status, created_at_nanos, updated_at_nanos, archived)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            ON CONFLICT (owner_user_id, normalized_name) DO NOTHING
        "#;
        let res = sqlx::query(query)
            .bind(&repo.id)
            .bind(&repo.owner_user_id)
            .bind(&repo.owner_username)
            .bind(&repo.name)
            .bind(&repo.normalized_name)
            .bind(&repo.description)
            .bind(repo.is_private)
            .bind(&repo.default_branch)
            .bind(&repo.native_repo_id)
            .bind(format!("{:?}", repo.durability_policy))
            .bind(repo.provisioning_status.as_str())
            .bind(repo.created_at_nanos)
            .bind(repo.updated_at_nanos)
            .bind(repo.archived)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if res.rows_affected() == 0 {
            return Err(ForgeError::Conflict(format!("Repository '{}/{}' already exists", repo.owner_username, repo.name)));
        }
        Ok(repo)
    }

    async fn get_repository_by_id(&self, id: &str) -> Result<Option<ForgeRepository>, ForgeError> {
        let query = "SELECT id, owner_user_id, owner_username, name, normalized_name, description, is_private, default_branch, native_repo_id, durability_policy, provisioning_status, created_at_nanos, updated_at_nanos, archived FROM forge_repositories WHERE id = $1";
        let row = sqlx::query(query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            let policy_str: String = r.get(9);
            let policy = match policy_str.as_str() {
                "SingleCopy" | "SINGLE_COPY" => spine_cluster::durability::DurabilityPolicy::SingleCopy,
                "TwoCopies" | "TWO_COPIES" => spine_cluster::durability::DurabilityPolicy::TwoCopies,
                _ => spine_cluster::durability::DurabilityPolicy::ThreeCopies,
            };
            let status_str: String = r.get(10);
            Ok(Some(ForgeRepository {
                id: r.get(0),
                owner_user_id: r.get(1),
                owner_username: r.get(2),
                name: r.get(3),
                normalized_name: r.get(4),
                description: r.get(5),
                is_private: r.get(6),
                default_branch: r.get(7),
                native_repo_id: r.get(8),
                durability_policy: policy,
                provisioning_status: ProvisioningStatus::from_str_lossy(&status_str),
                created_at_nanos: r.get(11),
                updated_at_nanos: r.get(12),
                archived: r.get(13),
            }))
        } else {
            Ok(None)
        }
    }

    async fn get_repository_by_owner_name(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<Option<ForgeRepository>, ForgeError> {
        let norm_owner = owner.trim().to_lowercase();
        let norm_name = name.trim().to_lowercase();
        let query = "SELECT id, owner_user_id, owner_username, name, normalized_name, description, is_private, default_branch, native_repo_id, durability_policy, provisioning_status, created_at_nanos, updated_at_nanos, archived FROM forge_repositories WHERE LOWER(owner_username) = $1 AND normalized_name = $2";
        let row = sqlx::query(query)
            .bind(&norm_owner)
            .bind(&norm_name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            let policy_str: String = r.get(9);
            let policy = match policy_str.as_str() {
                "SingleCopy" | "SINGLE_COPY" => spine_cluster::durability::DurabilityPolicy::SingleCopy,
                "TwoCopies" | "TWO_COPIES" => spine_cluster::durability::DurabilityPolicy::TwoCopies,
                _ => spine_cluster::durability::DurabilityPolicy::ThreeCopies,
            };
            let status_str: String = r.get(10);
            Ok(Some(ForgeRepository {
                id: r.get(0),
                owner_user_id: r.get(1),
                owner_username: r.get(2),
                name: r.get(3),
                normalized_name: r.get(4),
                description: r.get(5),
                is_private: r.get(6),
                default_branch: r.get(7),
                native_repo_id: r.get(8),
                durability_policy: policy,
                provisioning_status: ProvisioningStatus::from_str_lossy(&status_str),
                created_at_nanos: r.get(11),
                updated_at_nanos: r.get(12),
                archived: r.get(13),
            }))
        } else {
            Ok(None)
        }
    }

    async fn list_repositories_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<ForgeRepository>, ForgeError> {
        let query = r#"
            SELECT DISTINCT r.id, r.owner_user_id, r.owner_username, r.name, r.normalized_name, r.description, r.is_private, r.default_branch, r.native_repo_id, r.durability_policy, r.provisioning_status, r.created_at_nanos, r.updated_at_nanos, r.archived
            FROM forge_repositories r
            LEFT JOIN repository_permissions p ON p.repo_id = r.id AND p.user_id = $1
            WHERE r.archived = FALSE AND (r.is_private = FALSE OR r.owner_user_id = $1 OR p.user_id IS NOT NULL)
            ORDER BY r.updated_at_nanos DESC
        "#;
        let rows = sqlx::query(query)
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            let policy_str: String = r.get(9);
            let policy = match policy_str.as_str() {
                "SingleCopy" | "SINGLE_COPY" => spine_cluster::durability::DurabilityPolicy::SingleCopy,
                "TwoCopies" | "TWO_COPIES" => spine_cluster::durability::DurabilityPolicy::TwoCopies,
                _ => spine_cluster::durability::DurabilityPolicy::ThreeCopies,
            };
            let status_str: String = r.get(10);
            list.push(ForgeRepository {
                id: r.get(0),
                owner_user_id: r.get(1),
                owner_username: r.get(2),
                name: r.get(3),
                normalized_name: r.get(4),
                description: r.get(5),
                is_private: r.get(6),
                default_branch: r.get(7),
                native_repo_id: r.get(8),
                durability_policy: policy,
                provisioning_status: ProvisioningStatus::from_str_lossy(&status_str),
                created_at_nanos: r.get(11),
                updated_at_nanos: r.get(12),
                archived: r.get(13),
            });
        }
        Ok(list)
    }

    async fn list_public_repositories(&self) -> Result<Vec<ForgeRepository>, ForgeError> {
        let query = "SELECT id, owner_user_id, owner_username, name, normalized_name, description, is_private, default_branch, native_repo_id, durability_policy, provisioning_status, created_at_nanos, updated_at_nanos, archived FROM forge_repositories WHERE is_private = FALSE AND archived = FALSE ORDER BY updated_at_nanos DESC";
        let rows = sqlx::query(query)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            let policy_str: String = r.get(9);
            let policy = match policy_str.as_str() {
                "SingleCopy" | "SINGLE_COPY" => spine_cluster::durability::DurabilityPolicy::SingleCopy,
                "TwoCopies" | "TWO_COPIES" => spine_cluster::durability::DurabilityPolicy::TwoCopies,
                _ => spine_cluster::durability::DurabilityPolicy::ThreeCopies,
            };
            let status_str: String = r.get(10);
            list.push(ForgeRepository {
                id: r.get(0),
                owner_user_id: r.get(1),
                owner_username: r.get(2),
                name: r.get(3),
                normalized_name: r.get(4),
                description: r.get(5),
                is_private: r.get(6),
                default_branch: r.get(7),
                native_repo_id: r.get(8),
                durability_policy: policy,
                provisioning_status: ProvisioningStatus::from_str_lossy(&status_str),
                created_at_nanos: r.get(11),
                updated_at_nanos: r.get(12),
                archived: r.get(13),
            });
        }
        Ok(list)
    }

    async fn update_repository_status(
        &self,
        repo_id: &str,
        status: ProvisioningStatus,
        now_nanos: i64,
    ) -> Result<(), ForgeError> {
        let query = "UPDATE forge_repositories SET provisioning_status = $1, updated_at_nanos = $2 WHERE id = $3";
        let res = sqlx::query(query)
            .bind(status.as_str())
            .bind(now_nanos)
            .bind(repo_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if res.rows_affected() == 0 {
            return Err(ForgeError::NotFound("Repository not found".into()));
        }
        Ok(())
    }

    async fn delete_repository(&self, repo_id: &str) -> Result<(), ForgeError> {
        let query = "UPDATE forge_repositories SET archived = TRUE, provisioning_status = 'TOMBSTONED' WHERE id = $1";
        let res = sqlx::query(query)
            .bind(repo_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if res.rows_affected() == 0 {
            return Err(ForgeError::NotFound("Repository not found".into()));
        }
        Ok(())
    }

    async fn create_permission(
        &self,
        perm: RepositoryPermission,
    ) -> Result<RepositoryPermission, ForgeError> {
        let query = "INSERT INTO repository_permissions (id, repo_id, user_id, permission_role, created_at_nanos) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (repo_id, user_id) DO UPDATE SET permission_role = EXCLUDED.permission_role";
        sqlx::query(query)
            .bind(&perm.id)
            .bind(&perm.repo_id)
            .bind(&perm.user_id)
            .bind(perm.role.as_str())
            .bind(perm.created_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(perm)
    }

    async fn get_permission(
        &self,
        repo_id: &str,
        user_id: &str,
    ) -> Result<Option<PermissionRole>, ForgeError> {
        let query = "SELECT permission_role FROM repository_permissions WHERE repo_id = $1 AND user_id = $2";
        let row = sqlx::query(query)
            .bind(repo_id)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            let role_str: String = r.get(0);
            Ok(Some(PermissionRole::from_str_lossy(&role_str)))
        } else {
            Ok(None)
        }
    }

    async fn list_permissions_for_repo(
        &self,
        repo_id: &str,
    ) -> Result<Vec<RepositoryPermission>, ForgeError> {
        let query = "SELECT id, repo_id, user_id, permission_role, created_at_nanos FROM repository_permissions WHERE repo_id = $1";
        let rows = sqlx::query(query)
            .bind(repo_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            let role_str: String = r.get(3);
            list.push(RepositoryPermission {
                id: r.get(0),
                repo_id: r.get(1),
                user_id: r.get(2),
                role: PermissionRole::from_str_lossy(&role_str),
                created_at_nanos: r.get(4),
            });
        }
        Ok(list)
    }

    async fn delete_permission(&self, repo_id: &str, user_id: &str) -> Result<(), ForgeError> {
        let query = "DELETE FROM repository_permissions WHERE repo_id = $1 AND user_id = $2";
        sqlx::query(query)
            .bind(repo_id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn create_provisioning_job(
        &self,
        job: ProvisioningJob,
    ) -> Result<ProvisioningJob, ForgeError> {
        let query = "INSERT INTO repository_provisioning_jobs (id, repo_id, status, attempts, last_error, idempotency_key, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)";
        sqlx::query(query)
            .bind(&job.id)
            .bind(&job.repo_id)
            .bind(job.status.as_str())
            .bind(job.attempts as i32)
            .bind(&job.last_error)
            .bind(&job.idempotency_key)
            .bind(job.created_at_nanos)
            .bind(job.updated_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(job)
    }

    async fn get_next_pending_job(&self) -> Result<Option<ProvisioningJob>, ForgeError> {
        let query = "SELECT id, repo_id, status, attempts, last_error, idempotency_key, created_at_nanos, updated_at_nanos FROM repository_provisioning_jobs WHERE status IN ('PENDING', 'PROVISIONING') ORDER BY created_at_nanos ASC LIMIT 1";
        let row = sqlx::query(query)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            let status_str: String = r.get(2);
            let attempts: i32 = r.get(3);
            Ok(Some(ProvisioningJob {
                id: r.get(0),
                repo_id: r.get(1),
                status: ProvisioningStatus::from_str_lossy(&status_str),
                attempts: attempts as u32,
                last_error: r.get(4),
                idempotency_key: r.get(5),
                created_at_nanos: r.get(6),
                updated_at_nanos: r.get(7),
            }))
        } else {
            Ok(None)
        }
    }

    async fn update_job_status(
        &self,
        job_id: &str,
        status: ProvisioningStatus,
        last_error: Option<String>,
        now_nanos: i64,
    ) -> Result<(), ForgeError> {
        let query = "UPDATE repository_provisioning_jobs SET status = $1, last_error = $2, attempts = attempts + 1, updated_at_nanos = $3 WHERE id = $4";
        sqlx::query(query)
            .bind(status.as_str())
            .bind(last_error)
            .bind(now_nanos)
            .bind(job_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn record_audit_event(&self, event: AuditEvent) -> Result<(), ForgeError> {
        let query = "INSERT INTO audit_events (user_id, action, resource_type, resource_id, ip_address, metadata, timestamp_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7)";
        sqlx::query(query)
            .bind(&event.user_id)
            .bind(&event.action)
            .bind(&event.resource_type)
            .bind(&event.resource_id)
            .bind(&event.ip_address)
            .bind(&event.metadata)
            .bind(event.timestamp_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }
}
