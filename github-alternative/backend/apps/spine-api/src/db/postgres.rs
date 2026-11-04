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
            INSERT INTO forge_repositories (id, owner_user_id, owner_username, owner_org_id, name, normalized_name, description, is_private, default_branch, native_repo_id, durability_policy, provisioning_status, created_at_nanos, updated_at_nanos, archived)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
            ON CONFLICT (owner_user_id, normalized_name) DO NOTHING
        "#;
        let res = sqlx::query(query)
            .bind(&repo.id)
            .bind(&repo.owner_user_id)
            .bind(&repo.owner_username)
            .bind(&repo.owner_org_id)
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
        let query = "SELECT id, owner_user_id, owner_username, owner_org_id, name, normalized_name, description, is_private, default_branch, native_repo_id, durability_policy, provisioning_status, created_at_nanos, updated_at_nanos, archived FROM forge_repositories WHERE id = $1";
        let row = sqlx::query(query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            let policy_str: String = r.get(10);
            let policy = match policy_str.as_str() {
                "SingleCopy" | "SINGLE_COPY" => spine_cluster::durability::DurabilityPolicy::SingleCopy,
                "TwoCopies" | "TWO_COPIES" => spine_cluster::durability::DurabilityPolicy::TwoCopies,
                _ => spine_cluster::durability::DurabilityPolicy::ThreeCopies,
            };
            let status_str: String = r.get(11);
            Ok(Some(ForgeRepository {
                id: r.get(0),
                owner_user_id: r.get(1),
                owner_username: r.get(2),
                owner_org_id: r.get(3),
                name: r.get(4),
                normalized_name: r.get(5),
                description: r.get(6),
                is_private: r.get(7),
                default_branch: r.get(8),
                native_repo_id: r.get(9),
                durability_policy: policy,
                provisioning_status: ProvisioningStatus::from_str_lossy(&status_str),
                created_at_nanos: r.get(12),
                updated_at_nanos: r.get(13),
                archived: r.get(14),
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
        let query = "SELECT id, owner_user_id, owner_username, owner_org_id, name, normalized_name, description, is_private, default_branch, native_repo_id, durability_policy, provisioning_status, created_at_nanos, updated_at_nanos, archived FROM forge_repositories WHERE LOWER(owner_username) = $1 AND normalized_name = $2";
        let row = sqlx::query(query)
            .bind(&norm_owner)
            .bind(&norm_name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            let policy_str: String = r.get(10);
            let policy = match policy_str.as_str() {
                "SingleCopy" | "SINGLE_COPY" => spine_cluster::durability::DurabilityPolicy::SingleCopy,
                "TwoCopies" | "TWO_COPIES" => spine_cluster::durability::DurabilityPolicy::TwoCopies,
                _ => spine_cluster::durability::DurabilityPolicy::ThreeCopies,
            };
            let status_str: String = r.get(11);
            Ok(Some(ForgeRepository {
                id: r.get(0),
                owner_user_id: r.get(1),
                owner_username: r.get(2),
                owner_org_id: r.get(3),
                name: r.get(4),
                normalized_name: r.get(5),
                description: r.get(6),
                is_private: r.get(7),
                default_branch: r.get(8),
                native_repo_id: r.get(9),
                durability_policy: policy,
                provisioning_status: ProvisioningStatus::from_str_lossy(&status_str),
                created_at_nanos: r.get(12),
                updated_at_nanos: r.get(13),
                archived: r.get(14),
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
            SELECT DISTINCT r.id, r.owner_user_id, r.owner_username, r.owner_org_id, r.name, r.normalized_name, r.description, r.is_private, r.default_branch, r.native_repo_id, r.durability_policy, r.provisioning_status, r.created_at_nanos, r.updated_at_nanos, r.archived
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
            let policy_str: String = r.get(10);
            let policy = match policy_str.as_str() {
                "SingleCopy" | "SINGLE_COPY" => spine_cluster::durability::DurabilityPolicy::SingleCopy,
                "TwoCopies" | "TWO_COPIES" => spine_cluster::durability::DurabilityPolicy::TwoCopies,
                _ => spine_cluster::durability::DurabilityPolicy::ThreeCopies,
            };
            let status_str: String = r.get(11);
            list.push(ForgeRepository {
                id: r.get(0),
                owner_user_id: r.get(1),
                owner_username: r.get(2),
                owner_org_id: r.get(3),
                name: r.get(4),
                normalized_name: r.get(5),
                description: r.get(6),
                is_private: r.get(7),
                default_branch: r.get(8),
                native_repo_id: r.get(9),
                durability_policy: policy,
                provisioning_status: ProvisioningStatus::from_str_lossy(&status_str),
                created_at_nanos: r.get(12),
                updated_at_nanos: r.get(13),
                archived: r.get(14),
            });
        }
        Ok(list)
    }

    async fn list_repositories_for_org(
        &self,
        org_id: &str,
    ) -> Result<Vec<ForgeRepository>, ForgeError> {
        let query = "SELECT id, owner_user_id, owner_username, owner_org_id, name, normalized_name, description, is_private, default_branch, native_repo_id, durability_policy, provisioning_status, created_at_nanos, updated_at_nanos, archived FROM forge_repositories WHERE owner_org_id = $1 AND archived = FALSE ORDER BY updated_at_nanos DESC";
        let rows = sqlx::query(query)
            .bind(org_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            let policy_str: String = r.get(10);
            let policy = match policy_str.as_str() {
                "SingleCopy" | "SINGLE_COPY" => spine_cluster::durability::DurabilityPolicy::SingleCopy,
                "TwoCopies" | "TWO_COPIES" => spine_cluster::durability::DurabilityPolicy::TwoCopies,
                _ => spine_cluster::durability::DurabilityPolicy::ThreeCopies,
            };
            let status_str: String = r.get(11);
            list.push(ForgeRepository {
                id: r.get(0),
                owner_user_id: r.get(1),
                owner_username: r.get(2),
                owner_org_id: r.get(3),
                name: r.get(4),
                normalized_name: r.get(5),
                description: r.get(6),
                is_private: r.get(7),
                default_branch: r.get(8),
                native_repo_id: r.get(9),
                durability_policy: policy,
                provisioning_status: ProvisioningStatus::from_str_lossy(&status_str),
                created_at_nanos: r.get(12),
                updated_at_nanos: r.get(13),
                archived: r.get(14),
            });
        }
        Ok(list)
    }

    async fn list_public_repositories(&self) -> Result<Vec<ForgeRepository>, ForgeError> {
        let query = "SELECT id, owner_user_id, owner_username, owner_org_id, name, normalized_name, description, is_private, default_branch, native_repo_id, durability_policy, provisioning_status, created_at_nanos, updated_at_nanos, archived FROM forge_repositories WHERE is_private = FALSE AND archived = FALSE ORDER BY updated_at_nanos DESC";
        let rows = sqlx::query(query)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            let policy_str: String = r.get(10);
            let policy = match policy_str.as_str() {
                "SingleCopy" | "SINGLE_COPY" => spine_cluster::durability::DurabilityPolicy::SingleCopy,
                "TwoCopies" | "TWO_COPIES" => spine_cluster::durability::DurabilityPolicy::TwoCopies,
                _ => spine_cluster::durability::DurabilityPolicy::ThreeCopies,
            };
            let status_str: String = r.get(11);
            list.push(ForgeRepository {
                id: r.get(0),
                owner_user_id: r.get(1),
                owner_username: r.get(2),
                owner_org_id: r.get(3),
                name: r.get(4),
                normalized_name: r.get(5),
                description: r.get(6),
                is_private: r.get(7),
                default_branch: r.get(8),
                native_repo_id: r.get(9),
                durability_policy: policy,
                provisioning_status: ProvisioningStatus::from_str_lossy(&status_str),
                created_at_nanos: r.get(12),
                updated_at_nanos: r.get(13),
                archived: r.get(14),
            });
        }
        Ok(list)
    }

    async fn update_repository_owner_org(
        &self,
        repo_id: &str,
        org_id: &str,
        org_name: &str,
    ) -> Result<(), ForgeError> {
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let query = "UPDATE forge_repositories SET owner_org_id = $1, owner_username = $2, updated_at_nanos = $3 WHERE id = $4";
        let res = sqlx::query(query)
            .bind(org_id)
            .bind(org_name)
            .bind(now)
            .bind(repo_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if res.rows_affected() == 0 {
            return Err(ForgeError::NotFound("Repository not found".into()));
        }
        Ok(())
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

    // -------------------------------------------------------------------------
    // Organizations & Membership (Batch 09)
    // -------------------------------------------------------------------------
    async fn create_organization(&self, org: Organization) -> Result<Organization, ForgeError> {
        let query = r#"
            INSERT INTO organizations (id, name, normalized_name, display_name, description, avatar_url, owner_user_id, is_private, created_at_nanos, updated_at_nanos)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (normalized_name) DO NOTHING
        "#;
        let res = sqlx::query(query)
            .bind(&org.id)
            .bind(&org.name)
            .bind(&org.normalized_name)
            .bind(&org.display_name)
            .bind(&org.description)
            .bind(&org.avatar_url)
            .bind(&org.owner_user_id)
            .bind(org.is_private)
            .bind(org.created_at_nanos)
            .bind(org.updated_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if res.rows_affected() == 0 {
            return Err(ForgeError::Conflict(format!("Organization '{}' already exists", org.name)));
        }
        Ok(org)
    }

    async fn get_organization_by_id(&self, id: &str) -> Result<Option<Organization>, ForgeError> {
        let query = "SELECT id, name, normalized_name, display_name, description, avatar_url, owner_user_id, is_private, created_at_nanos, updated_at_nanos FROM organizations WHERE id = $1";
        let row = sqlx::query(query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            Ok(Some(Organization {
                id: r.get(0),
                name: r.get(1),
                normalized_name: r.get(2),
                display_name: r.get(3),
                description: r.get(4),
                avatar_url: r.get(5),
                owner_user_id: r.get(6),
                is_private: r.get(7),
                created_at_nanos: r.get(8),
                updated_at_nanos: r.get(9),
            }))
        } else {
            Ok(None)
        }
    }

    async fn get_organization_by_slug(&self, slug: &str) -> Result<Option<Organization>, ForgeError> {
        let norm = slug.trim().to_lowercase();
        let query = "SELECT id, name, normalized_name, display_name, description, avatar_url, owner_user_id, is_private, created_at_nanos, updated_at_nanos FROM organizations WHERE normalized_name = $1";
        let row = sqlx::query(query)
            .bind(&norm)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            Ok(Some(Organization {
                id: r.get(0),
                name: r.get(1),
                normalized_name: r.get(2),
                display_name: r.get(3),
                description: r.get(4),
                avatar_url: r.get(5),
                owner_user_id: r.get(6),
                is_private: r.get(7),
                created_at_nanos: r.get(8),
                updated_at_nanos: r.get(9),
            }))
        } else {
            Ok(None)
        }
    }

    async fn list_organizations_for_user(&self, user_id: &str) -> Result<Vec<Organization>, ForgeError> {
        let query = r#"
            SELECT DISTINCT o.id, o.name, o.normalized_name, o.display_name, o.description, o.avatar_url, o.owner_user_id, o.is_private, o.created_at_nanos, o.updated_at_nanos
            FROM organizations o
            LEFT JOIN organization_memberships m ON m.org_id = o.id AND m.user_id = $1
            WHERE o.owner_user_id = $1 OR m.user_id = $1
            ORDER BY o.name ASC
        "#;
        let rows = sqlx::query(query)
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| Organization {
            id: r.get(0),
            name: r.get(1),
            normalized_name: r.get(2),
            display_name: r.get(3),
            description: r.get(4),
            avatar_url: r.get(5),
            owner_user_id: r.get(6),
            is_private: r.get(7),
            created_at_nanos: r.get(8),
            updated_at_nanos: r.get(9),
        }).collect();
        Ok(list)
    }

    async fn update_organization(&self, org: Organization) -> Result<Organization, ForgeError> {
        let query = "UPDATE organizations SET display_name = $1, description = $2, avatar_url = $3, is_private = $4, updated_at_nanos = $5 WHERE id = $6";
        let res = sqlx::query(query)
            .bind(&org.display_name)
            .bind(&org.description)
            .bind(&org.avatar_url)
            .bind(org.is_private)
            .bind(org.updated_at_nanos)
            .bind(&org.id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if res.rows_affected() == 0 {
            return Err(ForgeError::NotFound("Organization not found".into()));
        }
        Ok(org)
    }

    async fn delete_organization(&self, org_id: &str) -> Result<(), ForgeError> {
        let query = "DELETE FROM organizations WHERE id = $1";
        let res = sqlx::query(query)
            .bind(org_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if res.rows_affected() == 0 {
            return Err(ForgeError::NotFound("Organization not found".into()));
        }
        Ok(())
    }

    async fn add_organization_member(&self, membership: OrganizationMembership) -> Result<(), ForgeError> {
        let query = "INSERT INTO organization_memberships (id, org_id, user_id, role, created_at_nanos) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (org_id, user_id) DO UPDATE SET role = EXCLUDED.role";
        sqlx::query(query)
            .bind(&membership.id)
            .bind(&membership.org_id)
            .bind(&membership.user_id)
            .bind(membership.role.as_str())
            .bind(membership.created_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn get_organization_membership(
        &self,
        org_id: &str,
        user_id: &str,
    ) -> Result<Option<OrganizationMembership>, ForgeError> {
        let query = "SELECT id, org_id, user_id, role, created_at_nanos FROM organization_memberships WHERE org_id = $1 AND user_id = $2";
        let row = sqlx::query(query)
            .bind(org_id)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            let role_str: String = r.get(3);
            Ok(Some(OrganizationMembership {
                id: r.get(0),
                org_id: r.get(1),
                user_id: r.get(2),
                role: OrganizationRole::from_str_lossy(&role_str),
                created_at_nanos: r.get(4),
            }))
        } else {
            Ok(None)
        }
    }

    async fn list_organization_members(&self, org_id: &str) -> Result<Vec<OrganizationMembership>, ForgeError> {
        let query = "SELECT id, org_id, user_id, role, created_at_nanos FROM organization_memberships WHERE org_id = $1 ORDER BY created_at_nanos ASC";
        let rows = sqlx::query(query)
            .bind(org_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| {
            let role_str: String = r.get(3);
            OrganizationMembership {
                id: r.get(0),
                org_id: r.get(1),
                user_id: r.get(2),
                role: OrganizationRole::from_str_lossy(&role_str),
                created_at_nanos: r.get(4),
            }
        }).collect();
        Ok(list)
    }

    async fn remove_organization_member(&self, org_id: &str, user_id: &str) -> Result<(), ForgeError> {
        let query = "DELETE FROM organization_memberships WHERE org_id = $1 AND user_id = $2";
        sqlx::query(query)
            .bind(org_id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn update_organization_member_role(
        &self,
        org_id: &str,
        user_id: &str,
        role: OrganizationRole,
    ) -> Result<(), ForgeError> {
        let query = "UPDATE organization_memberships SET role = $1 WHERE org_id = $2 AND user_id = $3";
        sqlx::query(query)
            .bind(role.as_str())
            .bind(org_id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn create_organization_invitation(&self, inv: OrganizationInvitation) -> Result<OrganizationInvitation, ForgeError> {
        let query = "INSERT INTO organization_invitations (id, org_id, inviter_user_id, invitee_email, role, token_hash, status, expires_at_nanos, created_at_nanos, responded_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)";
        sqlx::query(query)
            .bind(&inv.id)
            .bind(&inv.org_id)
            .bind(&inv.inviter_user_id)
            .bind(&inv.invitee_email)
            .bind(inv.role.as_str())
            .bind(&inv.token_hash)
            .bind(&inv.status)
            .bind(inv.expires_at_nanos)
            .bind(inv.created_at_nanos)
            .bind(inv.responded_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(inv)
    }

    async fn get_organization_invitation_by_token(
        &self,
        token_hash: &str,
    ) -> Result<Option<OrganizationInvitation>, ForgeError> {
        let query = "SELECT id, org_id, inviter_user_id, invitee_email, role, token_hash, status, expires_at_nanos, created_at_nanos, responded_at_nanos FROM organization_invitations WHERE token_hash = $1";
        let row = sqlx::query(query)
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            let role_str: String = r.get(4);
            Ok(Some(OrganizationInvitation {
                id: r.get(0),
                org_id: r.get(1),
                inviter_user_id: r.get(2),
                invitee_email: r.get(3),
                role: OrganizationRole::from_str_lossy(&role_str),
                token_hash: r.get(5),
                status: r.get(6),
                expires_at_nanos: r.get(7),
                created_at_nanos: r.get(8),
                responded_at_nanos: r.get(9),
            }))
        } else {
            Ok(None)
        }
    }

    async fn list_organization_invitations(&self, org_id: &str) -> Result<Vec<OrganizationInvitation>, ForgeError> {
        let query = "SELECT id, org_id, inviter_user_id, invitee_email, role, token_hash, status, expires_at_nanos, created_at_nanos, responded_at_nanos FROM organization_invitations WHERE org_id = $1 AND status = 'PENDING' ORDER BY created_at_nanos DESC";
        let rows = sqlx::query(query)
            .bind(org_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| {
            let role_str: String = r.get(4);
            OrganizationInvitation {
                id: r.get(0),
                org_id: r.get(1),
                inviter_user_id: r.get(2),
                invitee_email: r.get(3),
                role: OrganizationRole::from_str_lossy(&role_str),
                token_hash: r.get(5),
                status: r.get(6),
                expires_at_nanos: r.get(7),
                created_at_nanos: r.get(8),
                responded_at_nanos: r.get(9),
            }
        }).collect();
        Ok(list)
    }

    async fn update_organization_invitation_status(
        &self,
        id: &str,
        status: &str,
        now_nanos: i64,
    ) -> Result<(), ForgeError> {
        let query = "UPDATE organization_invitations SET status = $1, responded_at_nanos = $2 WHERE id = $3";
        sqlx::query(query)
            .bind(status)
            .bind(now_nanos)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Teams & Team Permissions (Batch 09)
    // -------------------------------------------------------------------------
    async fn create_team(&self, team: Team) -> Result<Team, ForgeError> {
        let query = "INSERT INTO teams (id, org_id, name, normalized_slug, description, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7)";
        sqlx::query(query)
            .bind(&team.id)
            .bind(&team.org_id)
            .bind(&team.name)
            .bind(&team.normalized_slug)
            .bind(&team.description)
            .bind(team.created_at_nanos)
            .bind(team.updated_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(team)
    }

    async fn get_team_by_id(&self, id: &str) -> Result<Option<Team>, ForgeError> {
        let query = "SELECT id, org_id, name, normalized_slug, description, created_at_nanos, updated_at_nanos FROM teams WHERE id = $1";
        let row = sqlx::query(query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            Ok(Some(Team {
                id: r.get(0),
                org_id: r.get(1),
                name: r.get(2),
                normalized_slug: r.get(3),
                description: r.get(4),
                created_at_nanos: r.get(5),
                updated_at_nanos: r.get(6),
            }))
        } else {
            Ok(None)
        }
    }

    async fn get_team_by_slug(&self, org_id: &str, slug: &str) -> Result<Option<Team>, ForgeError> {
        let norm = slug.trim().to_lowercase();
        let query = "SELECT id, org_id, name, normalized_slug, description, created_at_nanos, updated_at_nanos FROM teams WHERE org_id = $1 AND normalized_slug = $2";
        let row = sqlx::query(query)
            .bind(org_id)
            .bind(&norm)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            Ok(Some(Team {
                id: r.get(0),
                org_id: r.get(1),
                name: r.get(2),
                normalized_slug: r.get(3),
                description: r.get(4),
                created_at_nanos: r.get(5),
                updated_at_nanos: r.get(6),
            }))
        } else {
            Ok(None)
        }
    }

    async fn list_teams_for_org(&self, org_id: &str) -> Result<Vec<Team>, ForgeError> {
        let query = "SELECT id, org_id, name, normalized_slug, description, created_at_nanos, updated_at_nanos FROM teams WHERE org_id = $1 ORDER BY name ASC";
        let rows = sqlx::query(query)
            .bind(org_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| Team {
            id: r.get(0),
            org_id: r.get(1),
            name: r.get(2),
            normalized_slug: r.get(3),
            description: r.get(4),
            created_at_nanos: r.get(5),
            updated_at_nanos: r.get(6),
        }).collect();
        Ok(list)
    }

    async fn delete_team(&self, team_id: &str) -> Result<(), ForgeError> {
        let query = "DELETE FROM teams WHERE id = $1";
        sqlx::query(query)
            .bind(team_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn add_team_member(&self, membership: TeamMembership) -> Result<(), ForgeError> {
        let query = "INSERT INTO team_memberships (id, team_id, user_id, is_maintainer, created_at_nanos) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (team_id, user_id) DO UPDATE SET is_maintainer = EXCLUDED.is_maintainer";
        sqlx::query(query)
            .bind(&membership.id)
            .bind(&membership.team_id)
            .bind(&membership.user_id)
            .bind(membership.is_maintainer)
            .bind(membership.created_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn remove_team_member(&self, team_id: &str, user_id: &str) -> Result<(), ForgeError> {
        let query = "DELETE FROM team_memberships WHERE team_id = $1 AND user_id = $2";
        sqlx::query(query)
            .bind(team_id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_team_members(&self, team_id: &str) -> Result<Vec<TeamMembership>, ForgeError> {
        let query = "SELECT id, team_id, user_id, is_maintainer, created_at_nanos FROM team_memberships WHERE team_id = $1 ORDER BY created_at_nanos ASC";
        let rows = sqlx::query(query)
            .bind(team_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| TeamMembership {
            id: r.get(0),
            team_id: r.get(1),
            user_id: r.get(2),
            is_maintainer: r.get(3),
            created_at_nanos: r.get(4),
        }).collect();
        Ok(list)
    }

    async fn set_team_repository_permission(
        &self,
        perm: TeamRepositoryPermission,
    ) -> Result<(), ForgeError> {
        let query = "INSERT INTO team_repository_permissions (id, team_id, repo_id, permission_level, created_at_nanos) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (team_id, repo_id) DO UPDATE SET permission_level = EXCLUDED.permission_level";
        sqlx::query(query)
            .bind(&perm.id)
            .bind(&perm.team_id)
            .bind(&perm.repo_id)
            .bind(perm.permission_level.as_str())
            .bind(perm.created_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn get_team_repository_permission(
        &self,
        team_id: &str,
        repo_id: &str,
    ) -> Result<Option<TeamPermissionLevel>, ForgeError> {
        let query = "SELECT permission_level FROM team_repository_permissions WHERE team_id = $1 AND repo_id = $2";
        let row = sqlx::query(query)
            .bind(team_id)
            .bind(repo_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            let lvl_str: String = r.get(0);
            Ok(Some(TeamPermissionLevel::from_str_lossy(&lvl_str)))
        } else {
            Ok(None)
        }
    }

    async fn list_team_repository_permissions_for_repo(
        &self,
        repo_id: &str,
    ) -> Result<Vec<TeamRepositoryPermission>, ForgeError> {
        let query = "SELECT id, team_id, repo_id, permission_level, created_at_nanos FROM team_repository_permissions WHERE repo_id = $1";
        let rows = sqlx::query(query)
            .bind(repo_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| {
            let lvl_str: String = r.get(3);
            TeamRepositoryPermission {
                id: r.get(0),
                team_id: r.get(1),
                repo_id: r.get(2),
                permission_level: TeamPermissionLevel::from_str_lossy(&lvl_str),
                created_at_nanos: r.get(4),
            }
        }).collect();
        Ok(list)
    }

    async fn list_teams_for_user_in_org(
        &self,
        org_id: &str,
        user_id: &str,
    ) -> Result<Vec<Team>, ForgeError> {
        let query = r#"
            SELECT t.id, t.org_id, t.name, t.normalized_slug, t.description, t.created_at_nanos, t.updated_at_nanos
            FROM teams t
            JOIN team_memberships tm ON tm.team_id = t.id
            WHERE t.org_id = $1 AND tm.user_id = $2
            ORDER BY t.name ASC
        "#;
        let rows = sqlx::query(query)
            .bind(org_id)
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| Team {
            id: r.get(0),
            org_id: r.get(1),
            name: r.get(2),
            normalized_slug: r.get(3),
            description: r.get(4),
            created_at_nanos: r.get(5),
            updated_at_nanos: r.get(6),
        }).collect();
        Ok(list)
    }

    // -------------------------------------------------------------------------
    // Milestones & Labels (Batch 09)
    // -------------------------------------------------------------------------
    async fn create_milestone(&self, milestone: Milestone) -> Result<Milestone, ForgeError> {
        let query = "INSERT INTO milestones (id, repo_id, title, description, state, due_date_nanos, created_at_nanos, updated_at_nanos, closed_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)";
        sqlx::query(query)
            .bind(&milestone.id)
            .bind(&milestone.repo_id)
            .bind(&milestone.title)
            .bind(&milestone.description)
            .bind(&milestone.state)
            .bind(milestone.due_date_nanos)
            .bind(milestone.created_at_nanos)
            .bind(milestone.updated_at_nanos)
            .bind(milestone.closed_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(milestone)
    }

    async fn get_milestone_by_id(&self, id: &str) -> Result<Option<Milestone>, ForgeError> {
        let query = "SELECT id, repo_id, title, description, state, due_date_nanos, created_at_nanos, updated_at_nanos, closed_at_nanos FROM milestones WHERE id = $1";
        let row = sqlx::query(query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            Ok(Some(Milestone {
                id: r.get(0),
                repo_id: r.get(1),
                title: r.get(2),
                description: r.get(3),
                state: r.get(4),
                due_date_nanos: r.get(5),
                created_at_nanos: r.get(6),
                updated_at_nanos: r.get(7),
                closed_at_nanos: r.get(8),
            }))
        } else {
            Ok(None)
        }
    }

    async fn list_milestones_for_repo(&self, repo_id: &str) -> Result<Vec<Milestone>, ForgeError> {
        let query = "SELECT id, repo_id, title, description, state, due_date_nanos, created_at_nanos, updated_at_nanos, closed_at_nanos FROM milestones WHERE repo_id = $1 ORDER BY created_at_nanos DESC";
        let rows = sqlx::query(query)
            .bind(repo_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| Milestone {
            id: r.get(0),
            repo_id: r.get(1),
            title: r.get(2),
            description: r.get(3),
            state: r.get(4),
            due_date_nanos: r.get(5),
            created_at_nanos: r.get(6),
            updated_at_nanos: r.get(7),
            closed_at_nanos: r.get(8),
        }).collect();
        Ok(list)
    }

    async fn update_milestone(&self, milestone: Milestone) -> Result<Milestone, ForgeError> {
        let query = "UPDATE milestones SET title = $1, description = $2, state = $3, due_date_nanos = $4, updated_at_nanos = $5, closed_at_nanos = $6 WHERE id = $7";
        let res = sqlx::query(query)
            .bind(&milestone.title)
            .bind(&milestone.description)
            .bind(&milestone.state)
            .bind(milestone.due_date_nanos)
            .bind(milestone.updated_at_nanos)
            .bind(milestone.closed_at_nanos)
            .bind(&milestone.id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if res.rows_affected() == 0 {
            return Err(ForgeError::NotFound("Milestone not found".into()));
        }
        Ok(milestone)
    }

    async fn delete_milestone(&self, id: &str) -> Result<(), ForgeError> {
        let query = "DELETE FROM milestones WHERE id = $1";
        sqlx::query(query)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn create_label(&self, label: RepositoryLabel) -> Result<RepositoryLabel, ForgeError> {
        let query = "INSERT INTO repository_labels (id, repo_id, name, color, description, created_at_nanos) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (repo_id, name) DO UPDATE SET color = EXCLUDED.color, description = EXCLUDED.description";
        sqlx::query(query)
            .bind(&label.id)
            .bind(&label.repo_id)
            .bind(&label.name)
            .bind(&label.color)
            .bind(&label.description)
            .bind(label.created_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(label)
    }

    async fn list_labels_for_repo(&self, repo_id: &str) -> Result<Vec<RepositoryLabel>, ForgeError> {
        let query = "SELECT id, repo_id, name, color, description, created_at_nanos FROM repository_labels WHERE repo_id = $1 ORDER BY name ASC";
        let rows = sqlx::query(query)
            .bind(repo_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| RepositoryLabel {
            id: r.get(0),
            repo_id: r.get(1),
            name: r.get(2),
            color: r.get(3),
            description: r.get(4),
            created_at_nanos: r.get(5),
        }).collect();
        Ok(list)
    }

    async fn delete_label(&self, id: &str) -> Result<(), ForgeError> {
        let query = "DELETE FROM repository_labels WHERE id = $1";
        sqlx::query(query)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Issues & Comments (Batch 09)
    // -------------------------------------------------------------------------
    async fn create_issue(&self, mut issue: Issue) -> Result<Issue, ForgeError> {
        if issue.issue_number <= 0 {
            let max_row = sqlx::query("SELECT COALESCE(MAX(issue_number), 0) + 1 FROM issues WHERE repo_id = $1")
                .bind(&issue.repo_id)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
            issue.issue_number = max_row.get(0);
        }

        let query = r#"
            INSERT INTO issues (id, repo_id, issue_number, author_id, title, body, status, milestone_id, closed_by_user_id, closed_at_nanos, created_at_nanos, updated_at_nanos)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        "#;
        sqlx::query(query)
            .bind(&issue.id)
            .bind(&issue.repo_id)
            .bind(issue.issue_number)
            .bind(&issue.author_id)
            .bind(&issue.title)
            .bind(&issue.body)
            .bind(&issue.status)
            .bind(&issue.milestone_id)
            .bind(&issue.closed_by_user_id)
            .bind(issue.closed_at_nanos)
            .bind(issue.created_at_nanos)
            .bind(issue.updated_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(issue)
    }

    async fn get_issue_by_number(
        &self,
        repo_id: &str,
        issue_number: i64,
    ) -> Result<Option<Issue>, ForgeError> {
        let query = "SELECT id, repo_id, issue_number, author_id, title, body, status, milestone_id, closed_by_user_id, closed_at_nanos, created_at_nanos, updated_at_nanos FROM issues WHERE repo_id = $1 AND issue_number = $2";
        let row = sqlx::query(query)
            .bind(repo_id)
            .bind(issue_number)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            Ok(Some(Issue {
                id: r.get(0),
                repo_id: r.get(1),
                issue_number: r.get(2),
                author_id: r.get(3),
                title: r.get(4),
                body: r.get(5),
                status: r.get(6),
                milestone_id: r.get(7),
                closed_by_user_id: r.get(8),
                closed_at_nanos: r.get(9),
                created_at_nanos: r.get(10),
                updated_at_nanos: r.get(11),
            }))
        } else {
            Ok(None)
        }
    }

    async fn list_issues_for_repo(
        &self,
        repo_id: &str,
        status: Option<&str>,
    ) -> Result<Vec<Issue>, ForgeError> {
        let rows = if let Some(st) = status {
            let query = "SELECT id, repo_id, issue_number, author_id, title, body, status, milestone_id, closed_by_user_id, closed_at_nanos, created_at_nanos, updated_at_nanos FROM issues WHERE repo_id = $1 AND status = $2 ORDER BY issue_number DESC";
            sqlx::query(query)
                .bind(repo_id)
                .bind(st)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| ForgeError::Internal(e.to_string()))?
        } else {
            let query = "SELECT id, repo_id, issue_number, author_id, title, body, status, milestone_id, closed_by_user_id, closed_at_nanos, created_at_nanos, updated_at_nanos FROM issues WHERE repo_id = $1 ORDER BY issue_number DESC";
            sqlx::query(query)
                .bind(repo_id)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| ForgeError::Internal(e.to_string()))?
        };

        let list = rows.into_iter().map(|r| Issue {
            id: r.get(0),
            repo_id: r.get(1),
            issue_number: r.get(2),
            author_id: r.get(3),
            title: r.get(4),
            body: r.get(5),
            status: r.get(6),
            milestone_id: r.get(7),
            closed_by_user_id: r.get(8),
            closed_at_nanos: r.get(9),
            created_at_nanos: r.get(10),
            updated_at_nanos: r.get(11),
        }).collect();
        Ok(list)
    }

    async fn update_issue(&self, issue: Issue) -> Result<Issue, ForgeError> {
        let query = "UPDATE issues SET title = $1, body = $2, status = $3, milestone_id = $4, closed_by_user_id = $5, closed_at_nanos = $6, updated_at_nanos = $7 WHERE id = $8";
        let res = sqlx::query(query)
            .bind(&issue.title)
            .bind(&issue.body)
            .bind(&issue.status)
            .bind(&issue.milestone_id)
            .bind(&issue.closed_by_user_id)
            .bind(issue.closed_at_nanos)
            .bind(issue.updated_at_nanos)
            .bind(&issue.id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if res.rows_affected() == 0 {
            return Err(ForgeError::NotFound("Issue not found".into()));
        }
        Ok(issue)
    }

    async fn assign_issue(&self, issue_id: &str, user_id: &str) -> Result<(), ForgeError> {
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let id = format!("ia_{}", uuid::Uuid::new_v4().simple());
        let query = "INSERT INTO issue_assignees (id, issue_id, user_id, created_at_nanos) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING";
        sqlx::query(query)
            .bind(&id)
            .bind(issue_id)
            .bind(user_id)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn unassign_issue(&self, issue_id: &str, user_id: &str) -> Result<(), ForgeError> {
        let query = "DELETE FROM issue_assignees WHERE issue_id = $1 AND user_id = $2";
        sqlx::query(query)
            .bind(issue_id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_issue_assignees(&self, issue_id: &str) -> Result<Vec<String>, ForgeError> {
        let query = "SELECT user_id FROM issue_assignees WHERE issue_id = $1";
        let rows = sqlx::query(query)
            .bind(issue_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.get(0)).collect())
    }

    async fn add_issue_label(&self, issue_id: &str, label_id: &str) -> Result<(), ForgeError> {
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let id = format!("il_{}", uuid::Uuid::new_v4().simple());
        let query = "INSERT INTO issue_labels (id, issue_id, label_id, created_at_nanos) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING";
        sqlx::query(query)
            .bind(&id)
            .bind(issue_id)
            .bind(label_id)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn remove_issue_label(&self, issue_id: &str, label_id: &str) -> Result<(), ForgeError> {
        let query = "DELETE FROM issue_labels WHERE issue_id = $1 AND label_id = $2";
        sqlx::query(query)
            .bind(issue_id)
            .bind(label_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_issue_labels(&self, issue_id: &str) -> Result<Vec<String>, ForgeError> {
        let query = "SELECT label_id FROM issue_labels WHERE issue_id = $1";
        let rows = sqlx::query(query)
            .bind(issue_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.get(0)).collect())
    }

    async fn create_issue_comment(&self, comment: IssueComment) -> Result<IssueComment, ForgeError> {
        let query = "INSERT INTO issue_comments (id, issue_id, author_id, body, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $3, $4, $5, $6)";
        sqlx::query(query)
            .bind(&comment.id)
            .bind(&comment.issue_id)
            .bind(&comment.author_id)
            .bind(&comment.body)
            .bind(comment.created_at_nanos)
            .bind(comment.updated_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(comment)
    }

    async fn list_issue_comments(&self, issue_id: &str) -> Result<Vec<IssueComment>, ForgeError> {
        let query = "SELECT id, issue_id, author_id, body, created_at_nanos, updated_at_nanos FROM issue_comments WHERE issue_id = $1 ORDER BY created_at_nanos ASC";
        let rows = sqlx::query(query)
            .bind(issue_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| IssueComment {
            id: r.get(0),
            issue_id: r.get(1),
            author_id: r.get(2),
            body: r.get(3),
            created_at_nanos: r.get(4),
            updated_at_nanos: r.get(5),
        }).collect();
        Ok(list)
    }

    // -------------------------------------------------------------------------
    // Pull Requests, Reviews & Inline Comments (Batch 09)
    // -------------------------------------------------------------------------
    async fn create_pull_request(&self, mut pr: PullRequest) -> Result<PullRequest, ForgeError> {
        if pr.pr_number <= 0 {
            let max_row = sqlx::query("SELECT COALESCE(MAX(pr_number), 0) + 1 FROM pull_requests WHERE repo_id = $1")
                .bind(&pr.repo_id)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
            pr.pr_number = max_row.get(0);
        }

        let query = r#"
            INSERT INTO pull_requests (id, repo_id, pr_number, author_id, title, body, base_branch, head_branch, base_commit_id, head_commit_id, state, is_draft, merged_by_user_id, merge_commit_id, merge_strategy, closed_at_nanos, merged_at_nanos, created_at_nanos, updated_at_nanos)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19)
        "#;
        sqlx::query(query)
            .bind(&pr.id)
            .bind(&pr.repo_id)
            .bind(pr.pr_number)
            .bind(&pr.author_id)
            .bind(&pr.title)
            .bind(&pr.body)
            .bind(&pr.base_branch)
            .bind(&pr.head_branch)
            .bind(&pr.base_commit_id)
            .bind(&pr.head_commit_id)
            .bind(&pr.state)
            .bind(pr.is_draft)
            .bind(&pr.merged_by_user_id)
            .bind(&pr.merge_commit_id)
            .bind(&pr.merge_strategy)
            .bind(pr.closed_at_nanos)
            .bind(pr.merged_at_nanos)
            .bind(pr.created_at_nanos)
            .bind(pr.updated_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(pr)
    }

    async fn get_pull_request_by_number(
        &self,
        repo_id: &str,
        pr_number: i64,
    ) -> Result<Option<PullRequest>, ForgeError> {
        let query = "SELECT id, repo_id, pr_number, author_id, title, body, base_branch, head_branch, base_commit_id, head_commit_id, state, is_draft, merged_by_user_id, merge_commit_id, merge_strategy, closed_at_nanos, merged_at_nanos, created_at_nanos, updated_at_nanos FROM pull_requests WHERE repo_id = $1 AND pr_number = $2";
        let row = sqlx::query(query)
            .bind(repo_id)
            .bind(pr_number)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            Ok(Some(PullRequest {
                id: r.get(0),
                repo_id: r.get(1),
                pr_number: r.get(2),
                author_id: r.get(3),
                title: r.get(4),
                body: r.get(5),
                base_branch: r.get(6),
                head_branch: r.get(7),
                base_commit_id: r.get(8),
                head_commit_id: r.get(9),
                state: r.get(10),
                is_draft: r.get(11),
                merged_by_user_id: r.get(12),
                merge_commit_id: r.get(13),
                merge_strategy: r.get(14),
                closed_at_nanos: r.get(15),
                merged_at_nanos: r.get(16),
                created_at_nanos: r.get(17),
                updated_at_nanos: r.get(18),
            }))
        } else {
            Ok(None)
        }
    }

    async fn list_pull_requests_for_repo(
        &self,
        repo_id: &str,
        state: Option<&str>,
    ) -> Result<Vec<PullRequest>, ForgeError> {
        let rows = if let Some(st) = state {
            let query = "SELECT id, repo_id, pr_number, author_id, title, body, base_branch, head_branch, base_commit_id, head_commit_id, state, is_draft, merged_by_user_id, merge_commit_id, merge_strategy, closed_at_nanos, merged_at_nanos, created_at_nanos, updated_at_nanos FROM pull_requests WHERE repo_id = $1 AND state = $2 ORDER BY pr_number DESC";
            sqlx::query(query)
                .bind(repo_id)
                .bind(st)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| ForgeError::Internal(e.to_string()))?
        } else {
            let query = "SELECT id, repo_id, pr_number, author_id, title, body, base_branch, head_branch, base_commit_id, head_commit_id, state, is_draft, merged_by_user_id, merge_commit_id, merge_strategy, closed_at_nanos, merged_at_nanos, created_at_nanos, updated_at_nanos FROM pull_requests WHERE repo_id = $1 ORDER BY pr_number DESC";
            sqlx::query(query)
                .bind(repo_id)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| ForgeError::Internal(e.to_string()))?
        };

        let list = rows.into_iter().map(|r| PullRequest {
            id: r.get(0),
            repo_id: r.get(1),
            pr_number: r.get(2),
            author_id: r.get(3),
            title: r.get(4),
            body: r.get(5),
            base_branch: r.get(6),
            head_branch: r.get(7),
            base_commit_id: r.get(8),
            head_commit_id: r.get(9),
            state: r.get(10),
            is_draft: r.get(11),
            merged_by_user_id: r.get(12),
            merge_commit_id: r.get(13),
            merge_strategy: r.get(14),
            closed_at_nanos: r.get(15),
            merged_at_nanos: r.get(16),
            created_at_nanos: r.get(17),
            updated_at_nanos: r.get(18),
        }).collect();
        Ok(list)
    }

    async fn update_pull_request(&self, pr: PullRequest) -> Result<PullRequest, ForgeError> {
        let query = r#"
            UPDATE pull_requests
            SET title = $1, body = $2, base_commit_id = $3, head_commit_id = $4, state = $5, is_draft = $6,
                merged_by_user_id = $7, merge_commit_id = $8, merge_strategy = $9, closed_at_nanos = $10,
                merged_at_nanos = $11, updated_at_nanos = $12
            WHERE id = $13
        "#;
        let res = sqlx::query(query)
            .bind(&pr.title)
            .bind(&pr.body)
            .bind(&pr.base_commit_id)
            .bind(&pr.head_commit_id)
            .bind(&pr.state)
            .bind(pr.is_draft)
            .bind(&pr.merged_by_user_id)
            .bind(&pr.merge_commit_id)
            .bind(&pr.merge_strategy)
            .bind(pr.closed_at_nanos)
            .bind(pr.merged_at_nanos)
            .bind(pr.updated_at_nanos)
            .bind(&pr.id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if res.rows_affected() == 0 {
            return Err(ForgeError::NotFound("Pull request not found".into()));
        }
        Ok(pr)
    }

    async fn request_pr_reviewer(
        &self,
        pr_id: &str,
        reviewer_id: &str,
        requested_by: &str,
    ) -> Result<(), ForgeError> {
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let id = format!("prr_{}", uuid::Uuid::new_v4().simple());
        let query = "INSERT INTO pull_request_reviewers (id, pr_id, reviewer_id, requested_by_user_id, created_at_nanos) VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING";
        sqlx::query(query)
            .bind(&id)
            .bind(pr_id)
            .bind(reviewer_id)
            .bind(requested_by)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_pr_requested_reviewers(&self, pr_id: &str) -> Result<Vec<String>, ForgeError> {
        let query = "SELECT reviewer_id FROM pull_request_reviewers WHERE pr_id = $1";
        let rows = sqlx::query(query)
            .bind(pr_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.get(0)).collect())
    }

    async fn create_pr_review(&self, review: PullRequestReview) -> Result<PullRequestReview, ForgeError> {
        let query = "INSERT INTO pull_request_reviews (id, pr_id, reviewer_id, commit_id, state, body, dismissed_by_user_id, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)";
        sqlx::query(query)
            .bind(&review.id)
            .bind(&review.pr_id)
            .bind(&review.reviewer_id)
            .bind(&review.commit_id)
            .bind(&review.state)
            .bind(&review.body)
            .bind(&review.dismissed_by_user_id)
            .bind(review.created_at_nanos)
            .bind(review.updated_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(review)
    }

    async fn list_pr_reviews(&self, pr_id: &str) -> Result<Vec<PullRequestReview>, ForgeError> {
        let query = "SELECT id, pr_id, reviewer_id, commit_id, state, body, dismissed_by_user_id, created_at_nanos, updated_at_nanos FROM pull_request_reviews WHERE pr_id = $1 ORDER BY created_at_nanos ASC";
        let rows = sqlx::query(query)
            .bind(pr_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| PullRequestReview {
            id: r.get(0),
            pr_id: r.get(1),
            reviewer_id: r.get(2),
            commit_id: r.get(3),
            state: r.get(4),
            body: r.get(5),
            dismissed_by_user_id: r.get(6),
            created_at_nanos: r.get(7),
            updated_at_nanos: r.get(8),
        }).collect();
        Ok(list)
    }

    async fn dismiss_pr_review(
        &self,
        review_id: &str,
        dismissed_by: &str,
        now_nanos: i64,
    ) -> Result<(), ForgeError> {
        let query = "UPDATE pull_request_reviews SET state = 'DISMISSED', dismissed_by_user_id = $1, updated_at_nanos = $2 WHERE id = $3";
        let res = sqlx::query(query)
            .bind(dismissed_by)
            .bind(now_nanos)
            .bind(review_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if res.rows_affected() == 0 {
            return Err(ForgeError::NotFound("Review not found".into()));
        }
        Ok(())
    }

    async fn create_pr_review_comment(
        &self,
        comment: PullRequestReviewComment,
    ) -> Result<PullRequestReviewComment, ForgeError> {
        let query = "INSERT INTO pull_request_review_comments (id, pr_id, review_id, author_id, file_path, commit_id, diff_side, line_number, body, is_resolved, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)";
        sqlx::query(query)
            .bind(&comment.id)
            .bind(&comment.pr_id)
            .bind(&comment.review_id)
            .bind(&comment.author_id)
            .bind(&comment.file_path)
            .bind(&comment.commit_id)
            .bind(&comment.diff_side)
            .bind(comment.line_number)
            .bind(&comment.body)
            .bind(comment.is_resolved)
            .bind(comment.created_at_nanos)
            .bind(comment.updated_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(comment)
    }

    async fn list_pr_review_comments(
        &self,
        pr_id: &str,
    ) -> Result<Vec<PullRequestReviewComment>, ForgeError> {
        let query = "SELECT id, pr_id, review_id, author_id, file_path, commit_id, diff_side, line_number, body, is_resolved, created_at_nanos, updated_at_nanos FROM pull_request_review_comments WHERE pr_id = $1 ORDER BY created_at_nanos ASC";
        let rows = sqlx::query(query)
            .bind(pr_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| PullRequestReviewComment {
            id: r.get(0),
            pr_id: r.get(1),
            review_id: r.get(2),
            author_id: r.get(3),
            file_path: r.get(4),
            commit_id: r.get(5),
            diff_side: r.get(6),
            line_number: r.get(7),
            body: r.get(8),
            is_resolved: r.get(9),
            created_at_nanos: r.get(10),
            updated_at_nanos: r.get(11),
        }).collect();
        Ok(list)
    }

    async fn resolve_pr_review_comment(&self, comment_id: &str, resolved: bool) -> Result<(), ForgeError> {
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let query = "UPDATE pull_request_review_comments SET is_resolved = $1, updated_at_nanos = $2 WHERE id = $3";
        let res = sqlx::query(query)
            .bind(resolved)
            .bind(now)
            .bind(comment_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if res.rows_affected() == 0 {
            return Err(ForgeError::NotFound("Review comment not found".into()));
        }
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Branch Protection Rules (Batch 09)
    // -------------------------------------------------------------------------
    async fn set_branch_protection(
        &self,
        rule: BranchProtectionRule,
    ) -> Result<BranchProtectionRule, ForgeError> {
        let query = r#"
            INSERT INTO branch_protection_rules (id, repo_id, branch_pattern, require_pull_request, required_approvals, prevent_self_approval, require_changes_addressed, dismiss_stale_reviews_on_push, restrict_merges_to_admins, created_at_nanos, updated_at_nanos)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (repo_id, branch_pattern) DO UPDATE SET
                require_pull_request = EXCLUDED.require_pull_request,
                required_approvals = EXCLUDED.required_approvals,
                prevent_self_approval = EXCLUDED.prevent_self_approval,
                require_changes_addressed = EXCLUDED.require_changes_addressed,
                dismiss_stale_reviews_on_push = EXCLUDED.dismiss_stale_reviews_on_push,
                restrict_merges_to_admins = EXCLUDED.restrict_merges_to_admins,
                updated_at_nanos = EXCLUDED.updated_at_nanos
        "#;
        sqlx::query(query)
            .bind(&rule.id)
            .bind(&rule.repo_id)
            .bind(&rule.branch_pattern)
            .bind(rule.require_pull_request)
            .bind(rule.required_approvals)
            .bind(rule.prevent_self_approval)
            .bind(rule.require_changes_addressed)
            .bind(rule.dismiss_stale_reviews_on_push)
            .bind(rule.restrict_merges_to_admins)
            .bind(rule.created_at_nanos)
            .bind(rule.updated_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(rule)
    }

    async fn get_branch_protection(
        &self,
        repo_id: &str,
        branch: &str,
    ) -> Result<Option<BranchProtectionRule>, ForgeError> {
        let rules = self.list_branch_protection_rules(repo_id).await?;
        for r in rules {
            if r.branch_pattern == branch || r.branch_pattern == "*" {
                return Ok(Some(r));
            }
            if r.branch_pattern.ends_with('*') {
                let prefix = &r.branch_pattern[..r.branch_pattern.len() - 1];
                if branch.starts_with(prefix) {
                    return Ok(Some(r));
                }
            }
        }
        Ok(None)
    }

    async fn list_branch_protection_rules(
        &self,
        repo_id: &str,
    ) -> Result<Vec<BranchProtectionRule>, ForgeError> {
        let query = "SELECT id, repo_id, branch_pattern, require_pull_request, required_approvals, prevent_self_approval, require_changes_addressed, dismiss_stale_reviews_on_push, restrict_merges_to_admins, created_at_nanos, updated_at_nanos FROM branch_protection_rules WHERE repo_id = $1";
        let rows = sqlx::query(query)
            .bind(repo_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| BranchProtectionRule {
            id: r.get(0),
            repo_id: r.get(1),
            branch_pattern: r.get(2),
            require_pull_request: r.get(3),
            required_approvals: r.get(4),
            prevent_self_approval: r.get(5),
            require_changes_addressed: r.get(6),
            dismiss_stale_reviews_on_push: r.get(7),
            restrict_merges_to_admins: r.get(8),
            required_status_checks: Vec::new(),
            created_at_nanos: r.get(9),
            updated_at_nanos: r.get(10),
        }).collect();
        Ok(list)
    }

    async fn delete_branch_protection(&self, rule_id: &str) -> Result<(), ForgeError> {
        let query = "DELETE FROM branch_protection_rules WHERE id = $1";
        sqlx::query(query)
            .bind(rule_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Notifications & Activity Events (Batch 09)
    // -------------------------------------------------------------------------
    async fn create_notification(&self, notif: UserNotification) -> Result<UserNotification, ForgeError> {
        let query = "INSERT INTO user_notifications (id, user_id, reason, resource_type, resource_id, title, message, is_read, created_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)";
        sqlx::query(query)
            .bind(&notif.id)
            .bind(&notif.user_id)
            .bind(&notif.reason)
            .bind(&notif.resource_type)
            .bind(&notif.resource_id)
            .bind(&notif.title)
            .bind(&notif.message)
            .bind(notif.is_read)
            .bind(notif.created_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(notif)
    }

    async fn list_user_notifications(
        &self,
        user_id: &str,
        unread_only: bool,
    ) -> Result<Vec<UserNotification>, ForgeError> {
        let rows = if unread_only {
            let query = "SELECT id, user_id, reason, resource_type, resource_id, title, message, is_read, created_at_nanos FROM user_notifications WHERE user_id = $1 AND is_read = FALSE ORDER BY created_at_nanos DESC";
            sqlx::query(query)
                .bind(user_id)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| ForgeError::Internal(e.to_string()))?
        } else {
            let query = "SELECT id, user_id, reason, resource_type, resource_id, title, message, is_read, created_at_nanos FROM user_notifications WHERE user_id = $1 ORDER BY created_at_nanos DESC";
            sqlx::query(query)
                .bind(user_id)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| ForgeError::Internal(e.to_string()))?
        };

        let list = rows.into_iter().map(|r| UserNotification {
            id: r.get(0),
            user_id: r.get(1),
            reason: r.get(2),
            resource_type: r.get(3),
            resource_id: r.get(4),
            title: r.get(5),
            message: r.get(6),
            is_read: r.get(7),
            created_at_nanos: r.get(8),
        }).collect();
        Ok(list)
    }

    async fn mark_notification_read(&self, notif_id: &str, user_id: &str) -> Result<(), ForgeError> {
        let query = "UPDATE user_notifications SET is_read = TRUE WHERE id = $1 AND user_id = $2";
        let res = sqlx::query(query)
            .bind(notif_id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if res.rows_affected() == 0 {
            return Err(ForgeError::NotFound("Notification not found".into()));
        }
        Ok(())
    }

    async fn mark_all_notifications_read(&self, user_id: &str) -> Result<(), ForgeError> {
        let query = "UPDATE user_notifications SET is_read = TRUE WHERE user_id = $1";
        sqlx::query(query)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn record_activity_event(&self, event: RepositoryActivityEvent) -> Result<(), ForgeError> {
        let query = "INSERT INTO repository_activity_events (id, repo_id, actor_id, event_type, payload, created_at_nanos) VALUES ($1, $2, $3, $4, $5, $6)";
        sqlx::query(query)
            .bind(&event.id)
            .bind(&event.repo_id)
            .bind(&event.actor_id)
            .bind(&event.event_type)
            .bind(&event.payload)
            .bind(event.created_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_activity_events_for_repo(
        &self,
        repo_id: &str,
        limit: usize,
    ) -> Result<Vec<RepositoryActivityEvent>, ForgeError> {
        let query = "SELECT id, repo_id, actor_id, event_type, payload, created_at_nanos FROM repository_activity_events WHERE repo_id = $1 ORDER BY created_at_nanos DESC LIMIT $2";
        let rows = sqlx::query(query)
            .bind(repo_id)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| RepositoryActivityEvent {
            id: r.get(0),
            repo_id: r.get(1),
            actor_id: r.get(2),
            event_type: r.get(3),
            payload: r.get(4),
            created_at_nanos: r.get(5),
        }).collect();
        Ok(list)
    }

    // -------------------------------------------------------------
    // Batch 11: Artifact Storage, Releases, Namespaces, Packages
    // -------------------------------------------------------------

    async fn get_or_create_quota(&self, owner_type: &str, owner_id: &str) -> Result<ArtifactQuota, ForgeError> {
        let query = "SELECT id, owner_type, owner_id, logical_quota_bytes, used_logical_bytes, used_physical_bytes, created_at_nanos, updated_at_nanos FROM artifact_quotas WHERE owner_id = $1";
        let row = sqlx::query(query)
            .bind(owner_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(r) = row {
            return Ok(ArtifactQuota {
                id: r.get(0),
                owner_type: r.get(1),
                owner_id: r.get(2),
                logical_quota_bytes: r.get(3),
                used_logical_bytes: r.get(4),
                used_physical_bytes: r.get(5),
                created_at_nanos: r.get(6),
                updated_at_nanos: r.get(7),
            });
        }

        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as i64;
        let quota = ArtifactQuota {
            id: uuid::Uuid::new_v4().to_string(),
            owner_type: owner_type.to_string(),
            owner_id: owner_id.to_string(),
            logical_quota_bytes: 53_687_091_200, // 50 GB default
            used_logical_bytes: 0,
            used_physical_bytes: 0,
            created_at_nanos: now,
            updated_at_nanos: now,
        };

        let insert_q = r#"
            INSERT INTO artifact_quotas (id, owner_type, owner_id, logical_quota_bytes, used_logical_bytes, used_physical_bytes, created_at_nanos, updated_at_nanos)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (owner_id) DO UPDATE SET updated_at_nanos = EXCLUDED.updated_at_nanos
            RETURNING id, owner_type, owner_id, logical_quota_bytes, used_logical_bytes, used_physical_bytes, created_at_nanos, updated_at_nanos
        "#;
        let r = sqlx::query(insert_q)
            .bind(&quota.id)
            .bind(&quota.owner_type)
            .bind(&quota.owner_id)
            .bind(quota.logical_quota_bytes)
            .bind(quota.used_logical_bytes)
            .bind(quota.used_physical_bytes)
            .bind(quota.created_at_nanos)
            .bind(quota.updated_at_nanos)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(ArtifactQuota {
            id: r.get(0),
            owner_type: r.get(1),
            owner_id: r.get(2),
            logical_quota_bytes: r.get(3),
            used_logical_bytes: r.get(4),
            used_physical_bytes: r.get(5),
            created_at_nanos: r.get(6),
            updated_at_nanos: r.get(7),
        })
    }

    async fn update_quota_usage(&self, owner_id: &str, delta_logical: i64, delta_physical: i64) -> Result<(), ForgeError> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as i64;
        let query = r#"
            UPDATE artifact_quotas
            SET used_logical_bytes = GREATEST(0, used_logical_bytes + $1),
                used_physical_bytes = GREATEST(0, used_physical_bytes + $2),
                updated_at_nanos = $3
            WHERE owner_id = $4
        "#;
        sqlx::query(query)
            .bind(delta_logical)
            .bind(delta_physical)
            .bind(now)
            .bind(owner_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn create_upload_session(&self, session: ArtifactUploadSession) -> Result<ArtifactUploadSession, ForgeError> {
        let query = r#"
            INSERT INTO artifact_upload_sessions (id, artifact_id, owner_type, owner_id, state, total_size, chunk_size, expected_chunks, durability_policy, created_at_nanos, expires_at_nanos)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        "#;
        sqlx::query(query)
            .bind(&session.id)
            .bind(&session.artifact_id)
            .bind(&session.owner_type)
            .bind(&session.owner_id)
            .bind(&session.state)
            .bind(session.total_size)
            .bind(session.chunk_size)
            .bind(session.expected_chunks)
            .bind(&session.durability_policy)
            .bind(session.created_at_nanos)
            .bind(session.expires_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(session)
    }

    async fn get_upload_session(&self, session_id: &str) -> Result<Option<ArtifactUploadSession>, ForgeError> {
        let query = "SELECT id, artifact_id, owner_type, owner_id, state, total_size, chunk_size, expected_chunks, durability_policy, created_at_nanos, expires_at_nanos FROM artifact_upload_sessions WHERE id = $1";
        let row = sqlx::query(query)
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(row.map(|r| ArtifactUploadSession {
            id: r.get(0),
            artifact_id: r.get(1),
            owner_type: r.get(2),
            owner_id: r.get(3),
            state: r.get(4),
            total_size: r.get(5),
            chunk_size: r.get(6),
            expected_chunks: r.get(7),
            durability_policy: r.get(8),
            created_at_nanos: r.get(9),
            expires_at_nanos: r.get(10),
        }))
    }

    async fn update_upload_session_state(&self, session_id: &str, state: &str) -> Result<(), ForgeError> {
        let query = "UPDATE artifact_upload_sessions SET state = $1 WHERE id = $2";
        sqlx::query(query)
            .bind(state)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn save_artifact_manifest(&self, manifest: StoredArtifactManifest) -> Result<(), ForgeError> {
        let query = r#"
            INSERT INTO artifact_manifests (id, artifact_id, whole_object_hash, total_size, chunk_size, chunk_count, content_type, manifest_json, created_at_nanos)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (id) DO NOTHING
        "#;
        sqlx::query(query)
            .bind(&manifest.id)
            .bind(&manifest.artifact_id)
            .bind(&manifest.whole_object_hash)
            .bind(manifest.total_size)
            .bind(manifest.chunk_size)
            .bind(manifest.chunk_count)
            .bind(&manifest.content_type)
            .bind(&manifest.manifest_json)
            .bind(manifest.created_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn get_artifact_manifest(&self, manifest_id: &str) -> Result<Option<StoredArtifactManifest>, ForgeError> {
        let query = "SELECT id, artifact_id, whole_object_hash, total_size, chunk_size, chunk_count, content_type, manifest_json, created_at_nanos FROM artifact_manifests WHERE id = $1";
        let row = sqlx::query(query)
            .bind(manifest_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(row.map(|r| StoredArtifactManifest {
            id: r.get(0),
            artifact_id: r.get(1),
            whole_object_hash: r.get(2),
            total_size: r.get(3),
            chunk_size: r.get(4),
            chunk_count: r.get(5),
            content_type: r.get(6),
            manifest_json: r.get(7),
            created_at_nanos: r.get(8),
        }))
    }

    async fn create_release(&self, release: Release) -> Result<Release, ForgeError> {
        let query = r#"
            INSERT INTO releases (id, repo_id, tag_name, target_commit_id, title, body, is_draft, is_prerelease, author_id, created_at_nanos, published_at_nanos, updated_at_nanos)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        "#;
        sqlx::query(query)
            .bind(&release.id)
            .bind(&release.repo_id)
            .bind(&release.tag_name)
            .bind(&release.target_commit_id)
            .bind(&release.title)
            .bind(&release.body)
            .bind(release.is_draft)
            .bind(release.is_prerelease)
            .bind(&release.author_id)
            .bind(release.created_at_nanos)
            .bind(release.published_at_nanos)
            .bind(release.updated_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(release)
    }

    async fn get_release_by_id(&self, id: &str) -> Result<Option<Release>, ForgeError> {
        let query = "SELECT id, repo_id, tag_name, target_commit_id, title, body, is_draft, is_prerelease, author_id, created_at_nanos, published_at_nanos, updated_at_nanos FROM releases WHERE id = $1";
        let row = sqlx::query(query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(row.map(|r| Release {
            id: r.get(0),
            repo_id: r.get(1),
            tag_name: r.get(2),
            target_commit_id: r.get(3),
            title: r.get(4),
            body: r.get(5),
            is_draft: r.get(6),
            is_prerelease: r.get(7),
            author_id: r.get(8),
            created_at_nanos: r.get(9),
            published_at_nanos: r.get(10),
            updated_at_nanos: r.get(11),
        }))
    }

    async fn get_release_by_tag(&self, repo_id: &str, tag_name: &str) -> Result<Option<Release>, ForgeError> {
        let query = "SELECT id, repo_id, tag_name, target_commit_id, title, body, is_draft, is_prerelease, author_id, created_at_nanos, published_at_nanos, updated_at_nanos FROM releases WHERE repo_id = $1 AND tag_name = $2";
        let row = sqlx::query(query)
            .bind(repo_id)
            .bind(tag_name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(row.map(|r| Release {
            id: r.get(0),
            repo_id: r.get(1),
            tag_name: r.get(2),
            target_commit_id: r.get(3),
            title: r.get(4),
            body: r.get(5),
            is_draft: r.get(6),
            is_prerelease: r.get(7),
            author_id: r.get(8),
            created_at_nanos: r.get(9),
            published_at_nanos: r.get(10),
            updated_at_nanos: r.get(11),
        }))
    }

    async fn list_releases_for_repo(&self, repo_id: &str, include_drafts: bool) -> Result<Vec<Release>, ForgeError> {
        let query = "SELECT id, repo_id, tag_name, target_commit_id, title, body, is_draft, is_prerelease, author_id, created_at_nanos, published_at_nanos, updated_at_nanos FROM releases WHERE repo_id = $1 AND ($2 = true OR is_draft = false) ORDER BY created_at_nanos DESC";
        let rows = sqlx::query(query)
            .bind(repo_id)
            .bind(include_drafts)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| Release {
            id: r.get(0),
            repo_id: r.get(1),
            tag_name: r.get(2),
            target_commit_id: r.get(3),
            title: r.get(4),
            body: r.get(5),
            is_draft: r.get(6),
            is_prerelease: r.get(7),
            author_id: r.get(8),
            created_at_nanos: r.get(9),
            published_at_nanos: r.get(10),
            updated_at_nanos: r.get(11),
        }).collect();
        Ok(list)
    }

    async fn update_release(&self, release: Release) -> Result<Release, ForgeError> {
        let query = r#"
            UPDATE releases
            SET title = $1, body = $2, is_draft = $3, is_prerelease = $4, published_at_nanos = $5, updated_at_nanos = $6
            WHERE id = $7
        "#;
        sqlx::query(query)
            .bind(&release.title)
            .bind(&release.body)
            .bind(release.is_draft)
            .bind(release.is_prerelease)
            .bind(release.published_at_nanos)
            .bind(release.updated_at_nanos)
            .bind(&release.id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(release)
    }

    async fn delete_release(&self, id: &str) -> Result<(), ForgeError> {
        let query = "DELETE FROM releases WHERE id = $1";
        sqlx::query(query)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn create_release_asset(&self, asset: ReleaseAsset) -> Result<ReleaseAsset, ForgeError> {
        let query = r#"
            INSERT INTO release_assets (id, release_id, name, content_type, size_bytes, manifest_id, whole_hash, downloads_count, created_at_nanos)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        "#;
        sqlx::query(query)
            .bind(&asset.id)
            .bind(&asset.release_id)
            .bind(&asset.name)
            .bind(&asset.content_type)
            .bind(asset.size_bytes)
            .bind(&asset.manifest_id)
            .bind(&asset.whole_hash)
            .bind(asset.downloads_count)
            .bind(asset.created_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(asset)
    }

    async fn get_release_asset(&self, id: &str) -> Result<Option<ReleaseAsset>, ForgeError> {
        let query = "SELECT id, release_id, name, content_type, size_bytes, manifest_id, whole_hash, downloads_count, created_at_nanos FROM release_assets WHERE id = $1";
        let row = sqlx::query(query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(row.map(|r| ReleaseAsset {
            id: r.get(0),
            release_id: r.get(1),
            name: r.get(2),
            content_type: r.get(3),
            size_bytes: r.get(4),
            manifest_id: r.get(5),
            whole_hash: r.get(6),
            downloads_count: r.get(7),
            created_at_nanos: r.get(8),
        }))
    }

    async fn list_assets_for_release(&self, release_id: &str) -> Result<Vec<ReleaseAsset>, ForgeError> {
        let query = "SELECT id, release_id, name, content_type, size_bytes, manifest_id, whole_hash, downloads_count, created_at_nanos FROM release_assets WHERE release_id = $1 ORDER BY created_at_nanos ASC";
        let rows = sqlx::query(query)
            .bind(release_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| ReleaseAsset {
            id: r.get(0),
            release_id: r.get(1),
            name: r.get(2),
            content_type: r.get(3),
            size_bytes: r.get(4),
            manifest_id: r.get(5),
            whole_hash: r.get(6),
            downloads_count: r.get(7),
            created_at_nanos: r.get(8),
        }).collect();
        Ok(list)
    }

    async fn delete_release_asset(&self, id: &str) -> Result<(), ForgeError> {
        let query = "DELETE FROM release_assets WHERE id = $1";
        sqlx::query(query)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn increment_asset_downloads(&self, id: &str) -> Result<(), ForgeError> {
        let query = "UPDATE release_assets SET downloads_count = downloads_count + 1 WHERE id = $1";
        sqlx::query(query)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn create_artifact_namespace(&self, ns: ArtifactNamespace) -> Result<ArtifactNamespace, ForgeError> {
        let query = r#"
            INSERT INTO artifact_namespaces (id, name, owner_type, owner_id, is_private, description, created_at_nanos, updated_at_nanos)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#;
        sqlx::query(query)
            .bind(&ns.id)
            .bind(&ns.name)
            .bind(&ns.owner_type)
            .bind(&ns.owner_id)
            .bind(ns.is_private)
            .bind(&ns.description)
            .bind(ns.created_at_nanos)
            .bind(ns.updated_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(ns)
    }

    async fn get_artifact_namespace_by_name(&self, name: &str) -> Result<Option<ArtifactNamespace>, ForgeError> {
        let query = "SELECT id, name, owner_type, owner_id, is_private, description, created_at_nanos, updated_at_nanos FROM artifact_namespaces WHERE name = $1";
        let row = sqlx::query(query)
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(row.map(|r| ArtifactNamespace {
            id: r.get(0),
            name: r.get(1),
            owner_type: r.get(2),
            owner_id: r.get(3),
            is_private: r.get(4),
            description: r.get(5),
            created_at_nanos: r.get(6),
            updated_at_nanos: r.get(7),
        }))
    }

    async fn list_artifact_namespaces(&self) -> Result<Vec<ArtifactNamespace>, ForgeError> {
        let query = "SELECT id, name, owner_type, owner_id, is_private, description, created_at_nanos, updated_at_nanos FROM artifact_namespaces ORDER BY name ASC";
        let rows = sqlx::query(query)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| ArtifactNamespace {
            id: r.get(0),
            name: r.get(1),
            owner_type: r.get(2),
            owner_id: r.get(3),
            is_private: r.get(4),
            description: r.get(5),
            created_at_nanos: r.get(6),
            updated_at_nanos: r.get(7),
        }).collect();
        Ok(list)
    }

    async fn create_artifact_version(&self, ver: ArtifactVersion) -> Result<ArtifactVersion, ForgeError> {
        let query = r#"
            INSERT INTO artifact_versions (id, namespace_id, artifact_name, version, manifest_id, whole_hash, size_bytes, created_by_user_id, created_at_nanos)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        "#;
        sqlx::query(query)
            .bind(&ver.id)
            .bind(&ver.namespace_id)
            .bind(&ver.artifact_name)
            .bind(&ver.version)
            .bind(&ver.manifest_id)
            .bind(&ver.whole_hash)
            .bind(ver.size_bytes)
            .bind(&ver.created_by_user_id)
            .bind(ver.created_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(ver)
    }

    async fn get_artifact_version(&self, namespace_id: &str, artifact_name: &str, version: &str) -> Result<Option<ArtifactVersion>, ForgeError> {
        let query = "SELECT id, namespace_id, artifact_name, version, manifest_id, whole_hash, size_bytes, created_by_user_id, created_at_nanos FROM artifact_versions WHERE namespace_id = $1 AND artifact_name = $2 AND version = $3";
        let row = sqlx::query(query)
            .bind(namespace_id)
            .bind(artifact_name)
            .bind(version)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(row.map(|r| ArtifactVersion {
            id: r.get(0),
            namespace_id: r.get(1),
            artifact_name: r.get(2),
            version: r.get(3),
            manifest_id: r.get(4),
            whole_hash: r.get(5),
            size_bytes: r.get(6),
            created_by_user_id: r.get(7),
            created_at_nanos: r.get(8),
        }))
    }

    async fn list_artifact_versions(&self, namespace_id: &str, artifact_name: &str) -> Result<Vec<ArtifactVersion>, ForgeError> {
        let query = "SELECT id, namespace_id, artifact_name, version, manifest_id, whole_hash, size_bytes, created_by_user_id, created_at_nanos FROM artifact_versions WHERE namespace_id = $1 AND artifact_name = $2 ORDER BY created_at_nanos DESC";
        let rows = sqlx::query(query)
            .bind(namespace_id)
            .bind(artifact_name)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| ArtifactVersion {
            id: r.get(0),
            namespace_id: r.get(1),
            artifact_name: r.get(2),
            version: r.get(3),
            manifest_id: r.get(4),
            whole_hash: r.get(5),
            size_bytes: r.get(6),
            created_by_user_id: r.get(7),
            created_at_nanos: r.get(8),
        }).collect();
        Ok(list)
    }

    async fn create_package(&self, pkg: Package) -> Result<Package, ForgeError> {
        let query = r#"
            INSERT INTO packages (id, ecosystem, name, owner_type, owner_id, is_private, description, created_at_nanos, updated_at_nanos)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        "#;
        sqlx::query(query)
            .bind(&pkg.id)
            .bind(&pkg.ecosystem)
            .bind(&pkg.name)
            .bind(&pkg.owner_type)
            .bind(&pkg.owner_id)
            .bind(pkg.is_private)
            .bind(&pkg.description)
            .bind(pkg.created_at_nanos)
            .bind(pkg.updated_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(pkg)
    }

    async fn get_package_by_name(&self, ecosystem: &str, name: &str) -> Result<Option<Package>, ForgeError> {
        let query = "SELECT id, ecosystem, name, owner_type, owner_id, is_private, description, created_at_nanos, updated_at_nanos FROM packages WHERE ecosystem = $1 AND name = $2";
        let row = sqlx::query(query)
            .bind(ecosystem)
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(row.map(|r| Package {
            id: r.get(0),
            ecosystem: r.get(1),
            name: r.get(2),
            owner_type: r.get(3),
            owner_id: r.get(4),
            is_private: r.get(5),
            description: r.get(6),
            created_at_nanos: r.get(7),
            updated_at_nanos: r.get(8),
        }))
    }

    async fn list_packages(&self, ecosystem: &str) -> Result<Vec<Package>, ForgeError> {
        let query = "SELECT id, ecosystem, name, owner_type, owner_id, is_private, description, created_at_nanos, updated_at_nanos FROM packages WHERE ecosystem = $1 ORDER BY name ASC";
        let rows = sqlx::query(query)
            .bind(ecosystem)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| Package {
            id: r.get(0),
            ecosystem: r.get(1),
            name: r.get(2),
            owner_type: r.get(3),
            owner_id: r.get(4),
            is_private: r.get(5),
            description: r.get(6),
            created_at_nanos: r.get(7),
            updated_at_nanos: r.get(8),
        }).collect();
        Ok(list)
    }

    async fn create_package_version(&self, ver: PackageVersion) -> Result<PackageVersion, ForgeError> {
        let query = r#"
            INSERT INTO package_versions (id, package_id, version, manifest_id, cksum, size_bytes, metadata_json, is_yanked, created_by_user_id, created_at_nanos)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        "#;
        sqlx::query(query)
            .bind(&ver.id)
            .bind(&ver.package_id)
            .bind(&ver.version)
            .bind(&ver.manifest_id)
            .bind(&ver.cksum)
            .bind(ver.size_bytes)
            .bind(&ver.metadata_json)
            .bind(ver.is_yanked)
            .bind(&ver.created_by_user_id)
            .bind(ver.created_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(ver)
    }

    async fn get_package_version(&self, package_id: &str, version: &str) -> Result<Option<PackageVersion>, ForgeError> {
        let query = "SELECT id, package_id, version, manifest_id, cksum, size_bytes, metadata_json, is_yanked, created_by_user_id, created_at_nanos FROM package_versions WHERE package_id = $1 AND version = $2";
        let row = sqlx::query(query)
            .bind(package_id)
            .bind(version)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(row.map(|r| PackageVersion {
            id: r.get(0),
            package_id: r.get(1),
            version: r.get(2),
            manifest_id: r.get(3),
            cksum: r.get(4),
            size_bytes: r.get(5),
            metadata_json: r.get(6),
            is_yanked: r.get(7),
            created_by_user_id: r.get(8),
            created_at_nanos: r.get(9),
        }))
    }

    async fn list_package_versions(&self, package_id: &str) -> Result<Vec<PackageVersion>, ForgeError> {
        let query = "SELECT id, package_id, version, manifest_id, cksum, size_bytes, metadata_json, is_yanked, created_by_user_id, created_at_nanos FROM package_versions WHERE package_id = $1 ORDER BY created_at_nanos ASC";
        let rows = sqlx::query(query)
            .bind(package_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| PackageVersion {
            id: r.get(0),
            package_id: r.get(1),
            version: r.get(2),
            manifest_id: r.get(3),
            cksum: r.get(4),
            size_bytes: r.get(5),
            metadata_json: r.get(6),
            is_yanked: r.get(7),
            created_by_user_id: r.get(8),
            created_at_nanos: r.get(9),
        }).collect();
        Ok(list)
    }

    async fn set_package_version_yanked(&self, package_id: &str, version: &str, yanked: bool) -> Result<(), ForgeError> {
        let query = "UPDATE package_versions SET is_yanked = $1 WHERE package_id = $2 AND version = $3";
        sqlx::query(query)
            .bind(yanked)
            .bind(package_id)
            .bind(version)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Webhooks (Batch 12)
    // -------------------------------------------------------------------------

    async fn create_webhook_subscription(&self, sub: WebhookSubscription) -> Result<WebhookSubscription, ForgeError> {
        let query = "INSERT INTO webhook_subscriptions (id, owner_type, owner_id, target_url, secret_hash, secret_ciphertext, events, is_active, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)";
        sqlx::query(query)
            .bind(&sub.id)
            .bind(&sub.owner_type)
            .bind(&sub.owner_id)
            .bind(&sub.target_url)
            .bind(&sub.secret_hash)
            .bind(&sub.secret_ciphertext)
            .bind(&sub.events)
            .bind(sub.is_active)
            .bind(sub.created_at_nanos)
            .bind(sub.updated_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(sub)
    }

    async fn get_webhook_subscription(&self, id: &str) -> Result<Option<WebhookSubscription>, ForgeError> {
        let query = "SELECT id, owner_type, owner_id, target_url, secret_hash, secret_ciphertext, events, is_active, created_at_nanos, updated_at_nanos FROM webhook_subscriptions WHERE id = $1";
        let row = sqlx::query(query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(row.map(|r| WebhookSubscription {
            id: r.get(0),
            owner_type: r.get(1),
            owner_id: r.get(2),
            target_url: r.get(3),
            secret_hash: r.get(4),
            secret_ciphertext: r.get(5),
            events: r.get(6),
            is_active: r.get(7),
            created_at_nanos: r.get(8),
            updated_at_nanos: r.get(9),
        }))
    }

    async fn list_webhook_subscriptions(&self, owner_type: &str, owner_id: &str) -> Result<Vec<WebhookSubscription>, ForgeError> {
        let query = "SELECT id, owner_type, owner_id, target_url, secret_hash, secret_ciphertext, events, is_active, created_at_nanos, updated_at_nanos FROM webhook_subscriptions WHERE owner_type = $1 AND owner_id = $2 ORDER BY created_at_nanos ASC";
        let rows = sqlx::query(query)
            .bind(owner_type)
            .bind(owner_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| WebhookSubscription {
            id: r.get(0),
            owner_type: r.get(1),
            owner_id: r.get(2),
            target_url: r.get(3),
            secret_hash: r.get(4),
            secret_ciphertext: r.get(5),
            events: r.get(6),
            is_active: r.get(7),
            created_at_nanos: r.get(8),
            updated_at_nanos: r.get(9),
        }).collect();
        Ok(list)
    }

    async fn update_webhook_subscription(&self, sub: WebhookSubscription) -> Result<WebhookSubscription, ForgeError> {
        let query = "UPDATE webhook_subscriptions SET target_url = $1, secret_hash = $2, secret_ciphertext = $3, events = $4, is_active = $5, updated_at_nanos = $6 WHERE id = $7";
        sqlx::query(query)
            .bind(&sub.target_url)
            .bind(&sub.secret_hash)
            .bind(&sub.secret_ciphertext)
            .bind(&sub.events)
            .bind(sub.is_active)
            .bind(sub.updated_at_nanos)
            .bind(&sub.id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(sub)
    }

    async fn delete_webhook_subscription(&self, id: &str) -> Result<(), ForgeError> {
        let query = "DELETE FROM webhook_subscriptions WHERE id = $1";
        sqlx::query(query)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn create_webhook_delivery(&self, delivery: WebhookDelivery) -> Result<WebhookDelivery, ForgeError> {
        let query = "INSERT INTO webhook_deliveries (id, subscription_id, event_type, payload, response_status, response_body, execution_duration_ms, attempt, status, delivered_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)";
        sqlx::query(query)
            .bind(&delivery.id)
            .bind(&delivery.subscription_id)
            .bind(&delivery.event_type)
            .bind(&delivery.payload)
            .bind(delivery.response_status)
            .bind(&delivery.response_body)
            .bind(delivery.execution_duration_ms)
            .bind(delivery.attempt)
            .bind(&delivery.status)
            .bind(delivery.delivered_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(delivery)
    }

    async fn list_webhook_deliveries(&self, subscription_id: &str, limit: i64) -> Result<Vec<WebhookDelivery>, ForgeError> {
        let query = "SELECT id, subscription_id, event_type, payload, response_status, response_body, execution_duration_ms, attempt, status, delivered_at_nanos FROM webhook_deliveries WHERE subscription_id = $1 ORDER BY delivered_at_nanos DESC LIMIT $2";
        let rows = sqlx::query(query)
            .bind(subscription_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| WebhookDelivery {
            id: r.get(0),
            subscription_id: r.get(1),
            event_type: r.get(2),
            payload: r.get(3),
            response_status: r.get(4),
            response_body: r.get(5),
            execution_duration_ms: r.get(6),
            attempt: r.get(7),
            status: r.get(8),
            delivered_at_nanos: r.get(9),
        }).collect();
        Ok(list)
    }

    async fn enqueue_webhook_outbox(&self, item: WebhookOutboxItem) -> Result<WebhookOutboxItem, ForgeError> {
        let query = "INSERT INTO webhook_outbox (id, event_type, owner_type, owner_id, payload, attempts, max_attempts, status, last_error, created_at_nanos, next_attempt_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)";
        sqlx::query(query)
            .bind(&item.id)
            .bind(&item.event_type)
            .bind(&item.owner_type)
            .bind(&item.owner_id)
            .bind(&item.payload)
            .bind(item.attempts)
            .bind(item.max_attempts)
            .bind(&item.status)
            .bind(&item.last_error)
            .bind(item.created_at_nanos)
            .bind(item.next_attempt_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(item)
    }

    async fn claim_pending_webhook_outbox(&self, limit: i64, now_nanos: i64) -> Result<Vec<WebhookOutboxItem>, ForgeError> {
        let query = "UPDATE webhook_outbox SET status = 'PROCESSING', attempts = attempts + 1 WHERE id IN (SELECT id FROM webhook_outbox WHERE status = 'PENDING' AND next_attempt_at_nanos <= $1 ORDER BY next_attempt_at_nanos ASC LIMIT $2 FOR UPDATE SKIP LOCKED) RETURNING id, event_type, owner_type, owner_id, payload, attempts, max_attempts, status, last_error, created_at_nanos, next_attempt_at_nanos";
        let rows = sqlx::query(query)
            .bind(now_nanos)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| WebhookOutboxItem {
            id: r.get(0),
            event_type: r.get(1),
            owner_type: r.get(2),
            owner_id: r.get(3),
            payload: r.get(4),
            attempts: r.get(5),
            max_attempts: r.get(6),
            status: r.get(7),
            last_error: r.get(8),
            created_at_nanos: r.get(9),
            next_attempt_at_nanos: r.get(10),
        }).collect();
        Ok(list)
    }

    async fn update_webhook_outbox_status(&self, id: &str, status: &str, error: Option<&str>, next_attempt_nanos: i64) -> Result<(), ForgeError> {
        let query = "UPDATE webhook_outbox SET status = $1, last_error = $2, next_attempt_at_nanos = $3 WHERE id = $4";
        sqlx::query(query)
            .bind(status)
            .bind(error)
            .bind(next_attempt_nanos)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Workflows, Runs, Jobs, Steps, Logs (Batch 12)
    // -------------------------------------------------------------------------

    async fn upsert_workflow_definition(&self, def: WorkflowDefinition) -> Result<WorkflowDefinition, ForgeError> {
        let query = "INSERT INTO workflow_definitions (id, repo_id, path, name, yaml_content, digest, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (repo_id, path) DO UPDATE SET name = EXCLUDED.name, yaml_content = EXCLUDED.yaml_content, digest = EXCLUDED.digest, updated_at_nanos = EXCLUDED.updated_at_nanos";
        sqlx::query(query)
            .bind(&def.id)
            .bind(&def.repo_id)
            .bind(&def.path)
            .bind(&def.name)
            .bind(&def.yaml_content)
            .bind(&def.digest)
            .bind(def.created_at_nanos)
            .bind(def.updated_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(def)
    }

    async fn get_workflow_definition(&self, repo_id: &str, path: &str) -> Result<Option<WorkflowDefinition>, ForgeError> {
        let query = "SELECT id, repo_id, path, name, yaml_content, digest, created_at_nanos, updated_at_nanos FROM workflow_definitions WHERE repo_id = $1 AND path = $2";
        let row = sqlx::query(query)
            .bind(repo_id)
            .bind(path)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(row.map(|r| WorkflowDefinition {
            id: r.get(0),
            repo_id: r.get(1),
            path: r.get(2),
            name: r.get(3),
            yaml_content: r.get(4),
            digest: r.get(5),
            created_at_nanos: r.get(6),
            updated_at_nanos: r.get(7),
        }))
    }

    async fn list_workflow_definitions(&self, repo_id: &str) -> Result<Vec<WorkflowDefinition>, ForgeError> {
        let query = "SELECT id, repo_id, path, name, yaml_content, digest, created_at_nanos, updated_at_nanos FROM workflow_definitions WHERE repo_id = $1 ORDER BY path ASC";
        let rows = sqlx::query(query)
            .bind(repo_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| WorkflowDefinition {
            id: r.get(0),
            repo_id: r.get(1),
            path: r.get(2),
            name: r.get(3),
            yaml_content: r.get(4),
            digest: r.get(5),
            created_at_nanos: r.get(6),
            updated_at_nanos: r.get(7),
        }).collect();
        Ok(list)
    }

    async fn create_workflow_run(&self, run: WorkflowRun) -> Result<WorkflowRun, ForgeError> {
        let query = "INSERT INTO workflow_runs (id, repo_id, workflow_id, workflow_name, commit_oid, ref_name, trigger_event, actor_id, status, conclusion, definition_yaml, definition_digest, created_at_nanos, started_at_nanos, completed_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)";
        sqlx::query(query)
            .bind(&run.id)
            .bind(&run.repo_id)
            .bind(&run.workflow_id)
            .bind(&run.workflow_name)
            .bind(&run.commit_oid)
            .bind(&run.ref_name)
            .bind(&run.trigger_event)
            .bind(&run.actor_id)
            .bind(&run.status)
            .bind(&run.conclusion)
            .bind(&run.definition_yaml)
            .bind(&run.definition_digest)
            .bind(run.created_at_nanos)
            .bind(run.started_at_nanos)
            .bind(run.completed_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(run)
    }

    async fn get_workflow_run(&self, id: &str) -> Result<Option<WorkflowRun>, ForgeError> {
        let query = "SELECT id, repo_id, workflow_id, workflow_name, commit_oid, ref_name, trigger_event, actor_id, status, conclusion, definition_yaml, definition_digest, created_at_nanos, started_at_nanos, completed_at_nanos FROM workflow_runs WHERE id = $1";
        let row = sqlx::query(query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(row.map(|r| WorkflowRun {
            id: r.get(0),
            repo_id: r.get(1),
            workflow_id: r.get(2),
            workflow_name: r.get(3),
            commit_oid: r.get(4),
            ref_name: r.get(5),
            trigger_event: r.get(6),
            actor_id: r.get(7),
            status: r.get(8),
            conclusion: r.get(9),
            definition_yaml: r.get(10),
            definition_digest: r.get(11),
            created_at_nanos: r.get(12),
            started_at_nanos: r.get(13),
            completed_at_nanos: r.get(14),
        }))
    }

    async fn list_workflow_runs(&self, repo_id: &str, limit: i64) -> Result<Vec<WorkflowRun>, ForgeError> {
        let query = "SELECT id, repo_id, workflow_id, workflow_name, commit_oid, ref_name, trigger_event, actor_id, status, conclusion, definition_yaml, definition_digest, created_at_nanos, started_at_nanos, completed_at_nanos FROM workflow_runs WHERE repo_id = $1 ORDER BY created_at_nanos DESC LIMIT $2";
        let rows = sqlx::query(query)
            .bind(repo_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| WorkflowRun {
            id: r.get(0),
            repo_id: r.get(1),
            workflow_id: r.get(2),
            workflow_name: r.get(3),
            commit_oid: r.get(4),
            ref_name: r.get(5),
            trigger_event: r.get(6),
            actor_id: r.get(7),
            status: r.get(8),
            conclusion: r.get(9),
            definition_yaml: r.get(10),
            definition_digest: r.get(11),
            created_at_nanos: r.get(12),
            started_at_nanos: r.get(13),
            completed_at_nanos: r.get(14),
        }).collect();
        Ok(list)
    }

    async fn update_workflow_run_status(
        &self,
        id: &str,
        status: &str,
        conclusion: Option<&str>,
        started_at: Option<i64>,
        completed_at: Option<i64>,
    ) -> Result<(), ForgeError> {
        let query = "UPDATE workflow_runs SET status = $1, conclusion = COALESCE($2, conclusion), started_at_nanos = COALESCE($3, started_at_nanos), completed_at_nanos = COALESCE($4, completed_at_nanos) WHERE id = $5";
        sqlx::query(query)
            .bind(status)
            .bind(conclusion)
            .bind(started_at)
            .bind(completed_at)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn create_workflow_job(&self, job: WorkflowJob) -> Result<WorkflowJob, ForgeError> {
        let query = "INSERT INTO workflow_jobs (id, run_id, repo_id, job_key, name, status, runner_id, runs_on, needs, attempt, fencing_token, lease_expires_at_nanos, exit_code, created_at_nanos, started_at_nanos, completed_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)";
        sqlx::query(query)
            .bind(&job.id)
            .bind(&job.run_id)
            .bind(&job.repo_id)
            .bind(&job.job_key)
            .bind(&job.name)
            .bind(&job.status)
            .bind(&job.runner_id)
            .bind(serde_json::to_value(&job.runs_on).unwrap_or_default())
            .bind(serde_json::to_value(&job.needs).unwrap_or_default())
            .bind(job.attempt)
            .bind(job.fencing_token)
            .bind(job.lease_expires_at_nanos)
            .bind(job.exit_code)
            .bind(job.created_at_nanos)
            .bind(job.started_at_nanos)
            .bind(job.completed_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(job)
    }

    async fn get_workflow_job(&self, id: &str) -> Result<Option<WorkflowJob>, ForgeError> {
        let query = "SELECT id, run_id, repo_id, job_key, name, status, runner_id, runs_on, needs, attempt, fencing_token, lease_expires_at_nanos, exit_code, created_at_nanos, started_at_nanos, completed_at_nanos FROM workflow_jobs WHERE id = $1";
        let row = sqlx::query(query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(row.map(|r| {
            let runs_on_val: serde_json::Value = r.get(7);
            let needs_val: serde_json::Value = r.get(8);
            WorkflowJob {
                id: r.get(0),
                run_id: r.get(1),
                repo_id: r.get(2),
                job_key: r.get(3),
                name: r.get(4),
                status: r.get(5),
                runner_id: r.get(6),
                runs_on: serde_json::from_value(runs_on_val).unwrap_or_default(),
                needs: serde_json::from_value(needs_val).unwrap_or_default(),
                attempt: r.get(9),
                fencing_token: r.get(10),
                lease_expires_at_nanos: r.get(11),
                exit_code: r.get(12),
                created_at_nanos: r.get(13),
                started_at_nanos: r.get(14),
                completed_at_nanos: r.get(15),
            }
        }))
    }

    async fn list_workflow_jobs(&self, run_id: &str) -> Result<Vec<WorkflowJob>, ForgeError> {
        let query = "SELECT id, run_id, repo_id, job_key, name, status, runner_id, runs_on, needs, attempt, fencing_token, lease_expires_at_nanos, exit_code, created_at_nanos, started_at_nanos, completed_at_nanos FROM workflow_jobs WHERE run_id = $1 ORDER BY created_at_nanos ASC";
        let rows = sqlx::query(query)
            .bind(run_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| {
            let runs_on_val: serde_json::Value = r.get(7);
            let needs_val: serde_json::Value = r.get(8);
            WorkflowJob {
                id: r.get(0),
                run_id: r.get(1),
                repo_id: r.get(2),
                job_key: r.get(3),
                name: r.get(4),
                status: r.get(5),
                runner_id: r.get(6),
                runs_on: serde_json::from_value(runs_on_val).unwrap_or_default(),
                needs: serde_json::from_value(needs_val).unwrap_or_default(),
                attempt: r.get(9),
                fencing_token: r.get(10),
                lease_expires_at_nanos: r.get(11),
                exit_code: r.get(12),
                created_at_nanos: r.get(13),
                started_at_nanos: r.get(14),
                completed_at_nanos: r.get(15),
            }
        }).collect();
        Ok(list)
    }

    async fn list_queued_workflow_jobs(&self, now_nanos: i64) -> Result<Vec<WorkflowJob>, ForgeError> {
        let query = "SELECT id, run_id, repo_id, job_key, name, status, runner_id, runs_on, needs, attempt, fencing_token, lease_expires_at_nanos, exit_code, created_at_nanos, started_at_nanos, completed_at_nanos FROM workflow_jobs WHERE status = 'QUEUED' OR (status = 'ASSIGNED' AND lease_expires_at_nanos < $1) ORDER BY created_at_nanos ASC";
        let rows = sqlx::query(query)
            .bind(now_nanos)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| {
            let runs_on_val: serde_json::Value = r.get(7);
            let needs_val: serde_json::Value = r.get(8);
            WorkflowJob {
                id: r.get(0),
                run_id: r.get(1),
                repo_id: r.get(2),
                job_key: r.get(3),
                name: r.get(4),
                status: r.get(5),
                runner_id: r.get(6),
                runs_on: serde_json::from_value(runs_on_val).unwrap_or_default(),
                needs: serde_json::from_value(needs_val).unwrap_or_default(),
                attempt: r.get(9),
                fencing_token: r.get(10),
                lease_expires_at_nanos: r.get(11),
                exit_code: r.get(12),
                created_at_nanos: r.get(13),
                started_at_nanos: r.get(14),
                completed_at_nanos: r.get(15),
            }
        }).collect();
        Ok(list)
    }

    async fn assign_workflow_job(
        &self,
        job_id: &str,
        runner_id: &str,
        fencing_token: i64,
        lease_expires_at: i64,
    ) -> Result<(), ForgeError> {
        let query = "UPDATE workflow_jobs SET status = 'ASSIGNED', runner_id = $1, fencing_token = $2, lease_expires_at_nanos = $3 WHERE id = $4";
        sqlx::query(query)
            .bind(runner_id)
            .bind(fencing_token)
            .bind(lease_expires_at)
            .bind(job_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn update_workflow_job_status(
        &self,
        job_id: &str,
        status: &str,
        exit_code: Option<i32>,
        started_at: Option<i64>,
        completed_at: Option<i64>,
    ) -> Result<(), ForgeError> {
        let query = "UPDATE workflow_jobs SET status = $1, exit_code = COALESCE($2, exit_code), started_at_nanos = COALESCE($3, started_at_nanos), completed_at_nanos = COALESCE($4, completed_at_nanos) WHERE id = $5";
        sqlx::query(query)
            .bind(status)
            .bind(exit_code)
            .bind(started_at)
            .bind(completed_at)
            .bind(job_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn create_workflow_step(&self, step: WorkflowStep) -> Result<WorkflowStep, ForgeError> {
        let query = "INSERT INTO workflow_steps (id, job_id, step_index, name, status, exit_code, started_at_nanos, completed_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)";
        sqlx::query(query)
            .bind(&step.id)
            .bind(&step.job_id)
            .bind(step.step_index)
            .bind(&step.name)
            .bind(&step.status)
            .bind(step.exit_code)
            .bind(step.started_at_nanos)
            .bind(step.completed_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(step)
    }

    async fn list_workflow_steps(&self, job_id: &str) -> Result<Vec<WorkflowStep>, ForgeError> {
        let query = "SELECT id, job_id, step_index, name, status, exit_code, started_at_nanos, completed_at_nanos FROM workflow_steps WHERE job_id = $1 ORDER BY step_index ASC";
        let rows = sqlx::query(query)
            .bind(job_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| WorkflowStep {
            id: r.get(0),
            job_id: r.get(1),
            step_index: r.get(2),
            name: r.get(3),
            status: r.get(4),
            exit_code: r.get(5),
            started_at_nanos: r.get(6),
            completed_at_nanos: r.get(7),
        }).collect();
        Ok(list)
    }

    async fn update_workflow_step(
        &self,
        step_id: &str,
        status: &str,
        exit_code: Option<i32>,
        started_at: Option<i64>,
        completed_at: Option<i64>,
    ) -> Result<(), ForgeError> {
        let query = "UPDATE workflow_steps SET status = $1, exit_code = COALESCE($2, exit_code), started_at_nanos = COALESCE($3, started_at_nanos), completed_at_nanos = COALESCE($4, completed_at_nanos) WHERE id = $5";
        sqlx::query(query)
            .bind(status)
            .bind(exit_code)
            .bind(started_at)
            .bind(completed_at)
            .bind(step_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn append_workflow_log(&self, log: WorkflowLogChunk) -> Result<(), ForgeError> {
        let query = "INSERT INTO workflow_logs (id, job_id, step_index, stream, content, timestamp_nanos) VALUES ($1, $2, $3, $4, $5, $6)";
        sqlx::query(query)
            .bind(&log.id)
            .bind(&log.job_id)
            .bind(log.step_index)
            .bind(&log.stream)
            .bind(&log.content)
            .bind(log.timestamp_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_workflow_logs(&self, job_id: &str) -> Result<Vec<WorkflowLogChunk>, ForgeError> {
        let query = "SELECT id, job_id, step_index, stream, content, timestamp_nanos FROM workflow_logs WHERE job_id = $1 ORDER BY timestamp_nanos ASC";
        let rows = sqlx::query(query)
            .bind(job_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| WorkflowLogChunk {
            id: r.get(0),
            job_id: r.get(1),
            step_index: r.get(2),
            stream: r.get(3),
            content: r.get(4),
            timestamp_nanos: r.get(5),
        }).collect();
        Ok(list)
    }

    // -------------------------------------------------------------------------
    // Runners (Batch 12)
    // -------------------------------------------------------------------------

    async fn register_runner(&self, runner: RunnerRegistration) -> Result<RunnerRegistration, ForgeError> {
        let query = "INSERT INTO runner_registrations (id, name, hostname, token_hash, os, arch, labels, max_concurrency, active_jobs, status, last_heartbeat_at_nanos, created_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)";
        sqlx::query(query)
            .bind(&runner.id)
            .bind(&runner.name)
            .bind(&runner.hostname)
            .bind(&runner.token_hash)
            .bind(&runner.os)
            .bind(&runner.arch)
            .bind(serde_json::to_value(&runner.labels).unwrap_or_default())
            .bind(runner.max_concurrency)
            .bind(runner.active_jobs)
            .bind(&runner.status)
            .bind(runner.last_heartbeat_at_nanos)
            .bind(runner.created_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(runner)
    }

    async fn get_runner_by_id(&self, id: &str) -> Result<Option<RunnerRegistration>, ForgeError> {
        let query = "SELECT id, name, hostname, token_hash, os, arch, labels, max_concurrency, active_jobs, status, last_heartbeat_at_nanos, created_at_nanos FROM runner_registrations WHERE id = $1";
        let row = sqlx::query(query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(row.map(|r| {
            let labels_val: serde_json::Value = r.get(6);
            RunnerRegistration {
                id: r.get(0),
                name: r.get(1),
                hostname: r.get(2),
                token_hash: r.get(3),
                os: r.get(4),
                arch: r.get(5),
                labels: serde_json::from_value(labels_val).unwrap_or_default(),
                max_concurrency: r.get(7),
                active_jobs: r.get(8),
                status: r.get(9),
                last_heartbeat_at_nanos: r.get(10),
                created_at_nanos: r.get(11),
            }
        }))
    }

    async fn get_runner_by_token_hash(&self, token_hash: &str) -> Result<Option<RunnerRegistration>, ForgeError> {
        let query = "SELECT id, name, hostname, token_hash, os, arch, labels, max_concurrency, active_jobs, status, last_heartbeat_at_nanos, created_at_nanos FROM runner_registrations WHERE token_hash = $1";
        let row = sqlx::query(query)
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(row.map(|r| {
            let labels_val: serde_json::Value = r.get(6);
            RunnerRegistration {
                id: r.get(0),
                name: r.get(1),
                hostname: r.get(2),
                token_hash: r.get(3),
                os: r.get(4),
                arch: r.get(5),
                labels: serde_json::from_value(labels_val).unwrap_or_default(),
                max_concurrency: r.get(7),
                active_jobs: r.get(8),
                status: r.get(9),
                last_heartbeat_at_nanos: r.get(10),
                created_at_nanos: r.get(11),
            }
        }))
    }

    async fn list_runners(&self) -> Result<Vec<RunnerRegistration>, ForgeError> {
        let query = "SELECT id, name, hostname, token_hash, os, arch, labels, max_concurrency, active_jobs, status, last_heartbeat_at_nanos, created_at_nanos FROM runner_registrations ORDER BY name ASC";
        let rows = sqlx::query(query)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| {
            let labels_val: serde_json::Value = r.get(6);
            RunnerRegistration {
                id: r.get(0),
                name: r.get(1),
                hostname: r.get(2),
                token_hash: r.get(3),
                os: r.get(4),
                arch: r.get(5),
                labels: serde_json::from_value(labels_val).unwrap_or_default(),
                max_concurrency: r.get(7),
                active_jobs: r.get(8),
                status: r.get(9),
                last_heartbeat_at_nanos: r.get(10),
                created_at_nanos: r.get(11),
            }
        }).collect();
        Ok(list)
    }

    async fn update_runner_heartbeat(&self, runner_id: &str, active_jobs: i32, now_nanos: i64) -> Result<(), ForgeError> {
        let query = "UPDATE runner_registrations SET active_jobs = $1, last_heartbeat_at_nanos = $2 WHERE id = $3";
        sqlx::query(query)
            .bind(active_jobs)
            .bind(now_nanos)
            .bind(runner_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn update_runner_status(&self, runner_id: &str, status: &str) -> Result<(), ForgeError> {
        let query = "UPDATE runner_registrations SET status = $1 WHERE id = $2";
        sqlx::query(query)
            .bind(status)
            .bind(runner_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Workflow Secrets & PR Check Results (Batch 12)
    // -------------------------------------------------------------------------

    async fn upsert_workflow_secret(&self, secret: WorkflowSecret) -> Result<WorkflowSecret, ForgeError> {
        let query = "INSERT INTO workflow_secrets (id, owner_type, owner_id, name, ciphertext, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (owner_type, owner_id, name) DO UPDATE SET ciphertext = EXCLUDED.ciphertext, updated_at_nanos = EXCLUDED.updated_at_nanos";
        sqlx::query(query)
            .bind(&secret.id)
            .bind(&secret.owner_type)
            .bind(&secret.owner_id)
            .bind(&secret.name)
            .bind(&secret.ciphertext)
            .bind(secret.created_at_nanos)
            .bind(secret.updated_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(secret)
    }

    async fn get_workflow_secret(&self, owner_type: &str, owner_id: &str, name: &str) -> Result<Option<WorkflowSecret>, ForgeError> {
        let query = "SELECT id, owner_type, owner_id, name, ciphertext, created_at_nanos, updated_at_nanos FROM workflow_secrets WHERE owner_type = $1 AND owner_id = $2 AND name = $3";
        let row = sqlx::query(query)
            .bind(owner_type)
            .bind(owner_id)
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(row.map(|r| WorkflowSecret {
            id: r.get(0),
            owner_type: r.get(1),
            owner_id: r.get(2),
            name: r.get(3),
            ciphertext: r.get(4),
            created_at_nanos: r.get(5),
            updated_at_nanos: r.get(6),
        }))
    }

    async fn list_workflow_secrets(&self, owner_type: &str, owner_id: &str) -> Result<Vec<WorkflowSecret>, ForgeError> {
        let query = "SELECT id, owner_type, owner_id, name, ciphertext, created_at_nanos, updated_at_nanos FROM workflow_secrets WHERE owner_type = $1 AND owner_id = $2 ORDER BY name ASC";
        let rows = sqlx::query(query)
            .bind(owner_type)
            .bind(owner_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| WorkflowSecret {
            id: r.get(0),
            owner_type: r.get(1),
            owner_id: r.get(2),
            name: r.get(3),
            ciphertext: r.get(4),
            created_at_nanos: r.get(5),
            updated_at_nanos: r.get(6),
        }).collect();
        Ok(list)
    }

    async fn delete_workflow_secret(&self, owner_type: &str, owner_id: &str, name: &str) -> Result<(), ForgeError> {
        let query = "DELETE FROM workflow_secrets WHERE owner_type = $1 AND owner_id = $2 AND name = $3";
        sqlx::query(query)
            .bind(owner_type)
            .bind(owner_id)
            .bind(name)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn upsert_workflow_check_result(&self, check: WorkflowCheckResult) -> Result<WorkflowCheckResult, ForgeError> {
        let query = "INSERT INTO workflow_check_results (id, repo_id, commit_oid, check_name, status, run_id, updated_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (repo_id, commit_oid, check_name) DO UPDATE SET status = EXCLUDED.status, run_id = EXCLUDED.run_id, updated_at_nanos = EXCLUDED.updated_at_nanos";
        sqlx::query(query)
            .bind(&check.id)
            .bind(&check.repo_id)
            .bind(&check.commit_oid)
            .bind(&check.check_name)
            .bind(&check.status)
            .bind(&check.run_id)
            .bind(check.updated_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(check)
    }

    async fn get_workflow_check_result(&self, repo_id: &str, commit_oid: &str, check_name: &str) -> Result<Option<WorkflowCheckResult>, ForgeError> {
        let query = "SELECT id, repo_id, commit_oid, check_name, status, run_id, updated_at_nanos FROM workflow_check_results WHERE repo_id = $1 AND commit_oid = $2 AND check_name = $3";
        let row = sqlx::query(query)
            .bind(repo_id)
            .bind(commit_oid)
            .bind(check_name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(row.map(|r| WorkflowCheckResult {
            id: r.get(0),
            repo_id: r.get(1),
            commit_oid: r.get(2),
            check_name: r.get(3),
            status: r.get(4),
            run_id: r.get(5),
            updated_at_nanos: r.get(6),
        }))
    }

    async fn list_workflow_check_results(&self, repo_id: &str, commit_oid: &str) -> Result<Vec<WorkflowCheckResult>, ForgeError> {
        let query = "SELECT id, repo_id, commit_oid, check_name, status, run_id, updated_at_nanos FROM workflow_check_results WHERE repo_id = $1 AND commit_oid = $2 ORDER BY check_name ASC";
        let rows = sqlx::query(query)
            .bind(repo_id)
            .bind(commit_oid)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let list = rows.into_iter().map(|r| WorkflowCheckResult {
            id: r.get(0),
            repo_id: r.get(1),
            commit_oid: r.get(2),
            check_name: r.get(3),
            status: r.get(4),
            run_id: r.get(5),
            updated_at_nanos: r.get(6),
        }).collect();
        Ok(list)
    }

    // -------------------------------------------------------------------------
    // Discussions (Batch 13)
    // -------------------------------------------------------------------------
    async fn create_discussion_category(&self, cat: DiscussionCategory) -> Result<DiscussionCategory, ForgeError> {
        let query = "INSERT INTO discussion_categories (id, repo_id, name, slug, description, emoji, is_answerable, created_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)";
        sqlx::query(query)
            .bind(&cat.id)
            .bind(&cat.repo_id)
            .bind(&cat.name)
            .bind(&cat.slug)
            .bind(&cat.description)
            .bind(&cat.emoji)
            .bind(cat.is_answerable)
            .bind(cat.created_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(cat)
    }

    async fn get_discussion_category(&self, id: &str) -> Result<Option<DiscussionCategory>, ForgeError> {
        let query = "SELECT id, repo_id, name, slug, description, emoji, is_answerable, created_at_nanos FROM discussion_categories WHERE id = $1";
        let row = sqlx::query(query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(row.map(|r| DiscussionCategory {
            id: r.get(0),
            repo_id: r.get(1),
            name: r.get(2),
            slug: r.get(3),
            description: r.get(4),
            emoji: r.get(5),
            is_answerable: r.get(6),
            created_at_nanos: r.get(7),
        }))
    }

    async fn list_discussion_categories(&self, repo_id: &str) -> Result<Vec<DiscussionCategory>, ForgeError> {
        let query = "SELECT id, repo_id, name, slug, description, emoji, is_answerable, created_at_nanos FROM discussion_categories WHERE repo_id = $1 ORDER BY created_at_nanos ASC";
        let rows = sqlx::query(query)
            .bind(repo_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| DiscussionCategory {
            id: r.get(0),
            repo_id: r.get(1),
            name: r.get(2),
            slug: r.get(3),
            description: r.get(4),
            emoji: r.get(5),
            is_answerable: r.get(6),
            created_at_nanos: r.get(7),
        }).collect())
    }

    async fn create_discussion(&self, mut disc: Discussion) -> Result<Discussion, ForgeError> {
        let count_query = "SELECT COALESCE(MAX(number), 0) + 1 FROM discussions WHERE repo_id = $1";
        let row = sqlx::query(count_query)
            .bind(&disc.repo_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        disc.number = row.get(0);

        let query = "INSERT INTO discussions (id, repo_id, category_id, number, title, body, author_id, is_pinned, is_locked, answered_comment_id, comments_count, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)";
        sqlx::query(query)
            .bind(&disc.id)
            .bind(&disc.repo_id)
            .bind(&disc.category_id)
            .bind(disc.number)
            .bind(&disc.title)
            .bind(&disc.body)
            .bind(&disc.author_id)
            .bind(disc.is_pinned)
            .bind(disc.is_locked)
            .bind(&disc.answered_comment_id)
            .bind(disc.comments_count)
            .bind(disc.created_at_nanos)
            .bind(disc.updated_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(disc)
    }

    async fn get_discussion(&self, id: &str) -> Result<Option<Discussion>, ForgeError> {
        let query = "SELECT id, repo_id, category_id, number, title, body, author_id, is_pinned, is_locked, answered_comment_id, comments_count, created_at_nanos, updated_at_nanos FROM discussions WHERE id = $1";
        let row = sqlx::query(query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(row.map(|r| Discussion {
            id: r.get(0),
            repo_id: r.get(1),
            category_id: r.get(2),
            number: r.get(3),
            title: r.get(4),
            body: r.get(5),
            author_id: r.get(6),
            is_pinned: r.get(7),
            is_locked: r.get(8),
            answered_comment_id: r.get(9),
            comments_count: r.get(10),
            created_at_nanos: r.get(11),
            updated_at_nanos: r.get(12),
        }))
    }

    async fn get_discussion_by_number(&self, repo_id: &str, number: i32) -> Result<Option<Discussion>, ForgeError> {
        let query = "SELECT id, repo_id, category_id, number, title, body, author_id, is_pinned, is_locked, answered_comment_id, comments_count, created_at_nanos, updated_at_nanos FROM discussions WHERE repo_id = $1 AND number = $2";
        let row = sqlx::query(query)
            .bind(repo_id)
            .bind(number)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(row.map(|r| Discussion {
            id: r.get(0),
            repo_id: r.get(1),
            category_id: r.get(2),
            number: r.get(3),
            title: r.get(4),
            body: r.get(5),
            author_id: r.get(6),
            is_pinned: r.get(7),
            is_locked: r.get(8),
            answered_comment_id: r.get(9),
            comments_count: r.get(10),
            created_at_nanos: r.get(11),
            updated_at_nanos: r.get(12),
        }))
    }

    async fn list_discussions(&self, repo_id: &str, category_id: Option<&str>) -> Result<Vec<Discussion>, ForgeError> {
        let query = if let Some(cid) = category_id {
            sqlx::query("SELECT id, repo_id, category_id, number, title, body, author_id, is_pinned, is_locked, answered_comment_id, comments_count, created_at_nanos, updated_at_nanos FROM discussions WHERE repo_id = $1 AND category_id = $2 ORDER BY is_pinned DESC, created_at_nanos DESC")
                .bind(repo_id)
                .bind(cid)
        } else {
            sqlx::query("SELECT id, repo_id, category_id, number, title, body, author_id, is_pinned, is_locked, answered_comment_id, comments_count, created_at_nanos, updated_at_nanos FROM discussions WHERE repo_id = $1 ORDER BY is_pinned DESC, created_at_nanos DESC")
                .bind(repo_id)
        };
        let rows = query.fetch_all(&self.pool).await.map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| Discussion {
            id: r.get(0),
            repo_id: r.get(1),
            category_id: r.get(2),
            number: r.get(3),
            title: r.get(4),
            body: r.get(5),
            author_id: r.get(6),
            is_pinned: r.get(7),
            is_locked: r.get(8),
            answered_comment_id: r.get(9),
            comments_count: r.get(10),
            created_at_nanos: r.get(11),
            updated_at_nanos: r.get(12),
        }).collect())
    }

    async fn update_discussion(&self, disc: Discussion) -> Result<Discussion, ForgeError> {
        let query = "UPDATE discussions SET category_id = $1, title = $2, body = $3, is_pinned = $4, is_locked = $5, answered_comment_id = $6, comments_count = $7, updated_at_nanos = $8 WHERE id = $9";
        sqlx::query(query)
            .bind(&disc.category_id)
            .bind(&disc.title)
            .bind(&disc.body)
            .bind(disc.is_pinned)
            .bind(disc.is_locked)
            .bind(&disc.answered_comment_id)
            .bind(disc.comments_count)
            .bind(disc.updated_at_nanos)
            .bind(&disc.id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(disc)
    }

    async fn delete_discussion(&self, id: &str) -> Result<(), ForgeError> {
        sqlx::query("DELETE FROM discussions WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn create_discussion_comment(&self, comment: DiscussionComment) -> Result<DiscussionComment, ForgeError> {
        let query = "INSERT INTO discussion_comments (id, discussion_id, parent_comment_id, author_id, body, is_answer, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)";
        sqlx::query(query)
            .bind(&comment.id)
            .bind(&comment.discussion_id)
            .bind(&comment.parent_comment_id)
            .bind(&comment.author_id)
            .bind(&comment.body)
            .bind(comment.is_answer)
            .bind(comment.created_at_nanos)
            .bind(comment.updated_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let _ = sqlx::query("UPDATE discussions SET comments_count = comments_count + 1 WHERE id = $1")
            .bind(&comment.discussion_id)
            .execute(&self.pool)
            .await;
        Ok(comment)
    }

    async fn get_discussion_comment(&self, id: &str) -> Result<Option<DiscussionComment>, ForgeError> {
        let query = "SELECT id, discussion_id, parent_comment_id, author_id, body, is_answer, created_at_nanos, updated_at_nanos FROM discussion_comments WHERE id = $1";
        let row = sqlx::query(query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(row.map(|r| DiscussionComment {
            id: r.get(0),
            discussion_id: r.get(1),
            parent_comment_id: r.get(2),
            author_id: r.get(3),
            body: r.get(4),
            is_answer: r.get(5),
            created_at_nanos: r.get(6),
            updated_at_nanos: r.get(7),
        }))
    }

    async fn list_discussion_comments(&self, discussion_id: &str) -> Result<Vec<DiscussionComment>, ForgeError> {
        let query = "SELECT id, discussion_id, parent_comment_id, author_id, body, is_answer, created_at_nanos, updated_at_nanos FROM discussion_comments WHERE discussion_id = $1 ORDER BY created_at_nanos ASC";
        let rows = sqlx::query(query)
            .bind(discussion_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| DiscussionComment {
            id: r.get(0),
            discussion_id: r.get(1),
            parent_comment_id: r.get(2),
            author_id: r.get(3),
            body: r.get(4),
            is_answer: r.get(5),
            created_at_nanos: r.get(6),
            updated_at_nanos: r.get(7),
        }).collect())
    }

    async fn update_discussion_comment(&self, comment: DiscussionComment) -> Result<DiscussionComment, ForgeError> {
        let query = "UPDATE discussion_comments SET body = $1, is_answer = $2, updated_at_nanos = $3 WHERE id = $4";
        sqlx::query(query)
            .bind(&comment.body)
            .bind(comment.is_answer)
            .bind(comment.updated_at_nanos)
            .bind(&comment.id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(comment)
    }

    async fn delete_discussion_comment(&self, id: &str) -> Result<(), ForgeError> {
        if let Some(comment) = self.get_discussion_comment(id).await? {
            let _ = sqlx::query("DELETE FROM discussion_comments WHERE id = $1")
                .bind(id)
                .execute(&self.pool)
                .await;
            let _ = sqlx::query("UPDATE discussions SET comments_count = GREATEST(comments_count - 1, 0) WHERE id = $1")
                .bind(&comment.discussion_id)
                .execute(&self.pool)
                .await;
        }
        Ok(())
    }

    async fn add_discussion_reaction(&self, reaction: DiscussionReaction) -> Result<DiscussionReaction, ForgeError> {
        let query = "INSERT INTO discussion_reactions (id, subject_type, subject_id, user_id, reaction, created_at_nanos) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (subject_type, subject_id, user_id, reaction) DO NOTHING";
        sqlx::query(query)
            .bind(&reaction.id)
            .bind(&reaction.subject_type)
            .bind(&reaction.subject_id)
            .bind(&reaction.user_id)
            .bind(&reaction.reaction)
            .bind(reaction.created_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(reaction)
    }

    async fn remove_discussion_reaction(&self, subject_type: &str, subject_id: &str, user_id: &str, reaction: &str) -> Result<(), ForgeError> {
        let query = "DELETE FROM discussion_reactions WHERE subject_type = $1 AND subject_id = $2 AND user_id = $3 AND reaction = $4";
        sqlx::query(query)
            .bind(subject_type)
            .bind(subject_id)
            .bind(user_id)
            .bind(reaction)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_discussion_reactions(&self, subject_type: &str, subject_id: &str) -> Result<Vec<DiscussionReaction>, ForgeError> {
        let query = "SELECT id, subject_type, subject_id, user_id, reaction, created_at_nanos FROM discussion_reactions WHERE subject_type = $1 AND subject_id = $2";
        let rows = sqlx::query(query)
            .bind(subject_type)
            .bind(subject_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| DiscussionReaction {
            id: r.get(0),
            subject_type: r.get(1),
            subject_id: r.get(2),
            user_id: r.get(3),
            reaction: r.get(4),
            created_at_nanos: r.get(5),
        }).collect())
    }

    async fn set_discussion_subscription(&self, sub: DiscussionSubscription) -> Result<DiscussionSubscription, ForgeError> {
        let query = "INSERT INTO discussion_subscriptions (user_id, discussion_id, state, updated_at_nanos) VALUES ($1, $2, $3, $4) ON CONFLICT (user_id, discussion_id) DO UPDATE SET state = EXCLUDED.state, updated_at_nanos = EXCLUDED.updated_at_nanos";
        sqlx::query(query)
            .bind(&sub.user_id)
            .bind(&sub.discussion_id)
            .bind(&sub.state)
            .bind(sub.updated_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(sub)
    }

    async fn get_discussion_subscription(&self, user_id: &str, discussion_id: &str) -> Result<Option<DiscussionSubscription>, ForgeError> {
        let query = "SELECT user_id, discussion_id, state, updated_at_nanos FROM discussion_subscriptions WHERE user_id = $1 AND discussion_id = $2";
        let row = sqlx::query(query)
            .bind(user_id)
            .bind(discussion_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(row.map(|r| DiscussionSubscription {
            user_id: r.get(0),
            discussion_id: r.get(1),
            state: r.get(2),
            updated_at_nanos: r.get(3),
        }))
    }

    // -------------------------------------------------------------------------
    // Discovery, Topics, Languages, Profiles (Batch 13)
    // -------------------------------------------------------------------------
    async fn set_repository_topics(&self, repo_id: &str, topics: &[String]) -> Result<Vec<RepositoryTopic>, ForgeError> {
        sqlx::query("DELETE FROM repository_topics WHERE repo_id = $1")
            .bind(repo_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let mut created = Vec::new();
        for t in topics {
            let item = RepositoryTopic {
                id: uuid::Uuid::new_v4().to_string(),
                repo_id: repo_id.to_string(),
                topic: t.to_lowercase(),
                created_at_nanos: now,
            };
            sqlx::query("INSERT INTO repository_topics (id, repo_id, topic, created_at_nanos) VALUES ($1, $2, $3, $4)")
                .bind(&item.id)
                .bind(&item.repo_id)
                .bind(&item.topic)
                .bind(item.created_at_nanos)
                .execute(&self.pool)
                .await
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
            created.push(item);
        }
        Ok(created)
    }

    async fn list_repository_topics(&self, repo_id: &str) -> Result<Vec<RepositoryTopic>, ForgeError> {
        let query = "SELECT id, repo_id, topic, created_at_nanos FROM repository_topics WHERE repo_id = $1 ORDER BY topic ASC";
        let rows = sqlx::query(query)
            .bind(repo_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| RepositoryTopic {
            id: r.get(0),
            repo_id: r.get(1),
            topic: r.get(2),
            created_at_nanos: r.get(3),
        }).collect())
    }

    async fn list_popular_topics(&self, limit: usize) -> Result<Vec<PopularTopicDto>, ForgeError> {
        let query = "SELECT topic, COUNT(*)::bigint AS cnt FROM repository_topics GROUP BY topic ORDER BY cnt DESC LIMIT $1";
        let rows = sqlx::query(query)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| PopularTopicDto {
            topic: r.get(0),
            repository_count: r.get::<i64, _>(1) as usize,
        }).collect())
    }

    async fn list_repositories_by_topic(&self, topic: &str) -> Result<Vec<ForgeRepository>, ForgeError> {
        let query = "SELECT r.id, r.owner_user_id, r.owner_username, r.owner_org_id, r.name, r.normalized_name, r.description, r.is_private, r.default_branch, r.native_repo_id, r.durability_policy, r.provisioning_status, r.created_at_nanos, r.updated_at_nanos, r.archived FROM forge_repositories r JOIN repository_topics t ON r.id = t.repo_id WHERE t.topic = $1";
        let rows = sqlx::query(query)
            .bind(topic.to_lowercase())
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            let policy_str: String = r.get(10);
            let policy = match policy_str.as_str() {
                "SingleCopy" | "SINGLE_COPY" => spine_cluster::durability::DurabilityPolicy::SingleCopy,
                "TwoCopies" | "TWO_COPIES" => spine_cluster::durability::DurabilityPolicy::TwoCopies,
                _ => spine_cluster::durability::DurabilityPolicy::ThreeCopies,
            };
            let status_str: String = r.get(11);
            list.push(ForgeRepository {
                id: r.get(0),
                owner_user_id: r.get(1),
                owner_username: r.get(2),
                owner_org_id: r.get(3),
                name: r.get(4),
                normalized_name: r.get(5),
                description: r.get(6),
                is_private: r.get(7),
                default_branch: r.get(8),
                native_repo_id: r.get(9),
                durability_policy: policy,
                provisioning_status: ProvisioningStatus::from_str_lossy(&status_str),
                created_at_nanos: r.get(12),
                updated_at_nanos: r.get(13),
                archived: r.get(14),
            });
        }
        Ok(list)
    }

    async fn set_repository_languages(&self, repo_id: &str, langs: &[RepositoryLanguage]) -> Result<Vec<RepositoryLanguage>, ForgeError> {
        sqlx::query("DELETE FROM repository_languages WHERE repo_id = $1")
            .bind(repo_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        for l in langs {
            sqlx::query("INSERT INTO repository_languages (id, repo_id, language, bytes, percentage, updated_at_nanos) VALUES ($1, $2, $3, $4, $5, $6)")
                .bind(&l.id)
                .bind(&l.repo_id)
                .bind(&l.language)
                .bind(l.bytes)
                .bind(l.percentage)
                .bind(l.updated_at_nanos)
                .execute(&self.pool)
                .await
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
        }
        Ok(langs.to_vec())
    }

    async fn list_repository_languages(&self, repo_id: &str) -> Result<Vec<RepositoryLanguage>, ForgeError> {
        let query = "SELECT id, repo_id, language, bytes, percentage, updated_at_nanos FROM repository_languages WHERE repo_id = $1 ORDER BY bytes DESC";
        let rows = sqlx::query(query)
            .bind(repo_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| RepositoryLanguage {
            id: r.get(0),
            repo_id: r.get(1),
            language: r.get(2),
            bytes: r.get(3),
            percentage: r.get(4),
            updated_at_nanos: r.get(5),
        }).collect())
    }

    async fn get_extended_user_profile(&self, user_id: &str) -> Result<Option<ExtendedUserProfile>, ForgeError> {
        let query = "SELECT user_id, bio, company, location, website_url, twitter_username, profile_readme, pinned_repo_ids, updated_at_nanos FROM user_profiles WHERE user_id = $1";
        let row = sqlx::query(query)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(row.map(|r| {
            let pinned_raw: serde_json::Value = r.get(7);
            let pinned_repo_ids: Vec<String> = serde_json::from_value(pinned_raw).unwrap_or_default();
            ExtendedUserProfile {
                user_id: r.get(0),
                bio: r.get(1),
                company: r.get(2),
                location: r.get(3),
                website_url: r.get(4),
                twitter_username: r.get(5),
                profile_readme: r.get(6),
                pinned_repo_ids,
                updated_at_nanos: r.get(8),
            }
        }))
    }

    async fn upsert_extended_user_profile(&self, profile: ExtendedUserProfile) -> Result<ExtendedUserProfile, ForgeError> {
        let pinned_val = serde_json::to_value(&profile.pinned_repo_ids).unwrap_or(serde_json::json!([]));
        let query = "INSERT INTO user_profiles (user_id, bio, company, location, website_url, twitter_username, profile_readme, pinned_repo_ids, updated_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) ON CONFLICT (user_id) DO UPDATE SET bio = EXCLUDED.bio, company = EXCLUDED.company, location = EXCLUDED.location, website_url = EXCLUDED.website_url, twitter_username = EXCLUDED.twitter_username, profile_readme = EXCLUDED.profile_readme, pinned_repo_ids = EXCLUDED.pinned_repo_ids, updated_at_nanos = EXCLUDED.updated_at_nanos";
        sqlx::query(query)
            .bind(&profile.user_id)
            .bind(&profile.bio)
            .bind(&profile.company)
            .bind(&profile.location)
            .bind(&profile.website_url)
            .bind(&profile.twitter_username)
            .bind(&profile.profile_readme)
            .bind(pinned_val)
            .bind(profile.updated_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(profile)
    }

    // -------------------------------------------------------------------------
    // Security Center (Batch 13)
    // -------------------------------------------------------------------------
    async fn create_security_scan(&self, scan: SecurityScan) -> Result<SecurityScan, ForgeError> {
        let query = "INSERT INTO security_scans (id, repo_id, commit_oid, scan_type, status, findings_count, started_at_nanos, completed_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)";
        sqlx::query(query)
            .bind(&scan.id)
            .bind(&scan.repo_id)
            .bind(&scan.commit_oid)
            .bind(&scan.scan_type)
            .bind(&scan.status)
            .bind(scan.findings_count)
            .bind(scan.started_at_nanos)
            .bind(scan.completed_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(scan)
    }

    async fn list_security_scans(&self, repo_id: &str) -> Result<Vec<SecurityScan>, ForgeError> {
        let query = "SELECT id, repo_id, commit_oid, scan_type, status, findings_count, started_at_nanos, completed_at_nanos FROM security_scans WHERE repo_id = $1 ORDER BY started_at_nanos DESC";
        let rows = sqlx::query(query)
            .bind(repo_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| SecurityScan {
            id: r.get(0),
            repo_id: r.get(1),
            commit_oid: r.get(2),
            scan_type: r.get(3),
            status: r.get(4),
            findings_count: r.get(5),
            started_at_nanos: r.get(6),
            completed_at_nanos: r.get(7),
        }).collect())
    }

    async fn create_secret_scan_finding(&self, finding: SecretScanFinding) -> Result<SecretScanFinding, ForgeError> {
        let query = "INSERT INTO secret_scan_findings (id, repo_id, commit_oid, file_path, line_number, rule_id, rule_name, severity, safe_fingerprint, status, created_at_nanos, resolved_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)";
        sqlx::query(query)
            .bind(&finding.id)
            .bind(&finding.repo_id)
            .bind(&finding.commit_oid)
            .bind(&finding.file_path)
            .bind(finding.line_number)
            .bind(&finding.rule_id)
            .bind(&finding.rule_name)
            .bind(&finding.severity)
            .bind(&finding.safe_fingerprint)
            .bind(&finding.status)
            .bind(finding.created_at_nanos)
            .bind(finding.resolved_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(finding)
    }

    async fn list_secret_scan_findings(&self, repo_id: &str, status_filter: Option<&str>) -> Result<Vec<SecretScanFinding>, ForgeError> {
        let query = if let Some(st) = status_filter {
            sqlx::query("SELECT id, repo_id, commit_oid, file_path, line_number, rule_id, rule_name, severity, safe_fingerprint, status, created_at_nanos, resolved_at_nanos FROM secret_scan_findings WHERE repo_id = $1 AND status = $2 ORDER BY created_at_nanos DESC")
                .bind(repo_id)
                .bind(st)
        } else {
            sqlx::query("SELECT id, repo_id, commit_oid, file_path, line_number, rule_id, rule_name, severity, safe_fingerprint, status, created_at_nanos, resolved_at_nanos FROM secret_scan_findings WHERE repo_id = $1 ORDER BY created_at_nanos DESC")
                .bind(repo_id)
        };
        let rows = query.fetch_all(&self.pool).await.map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| SecretScanFinding {
            id: r.get(0),
            repo_id: r.get(1),
            commit_oid: r.get(2),
            file_path: r.get(3),
            line_number: r.get(4),
            rule_id: r.get(5),
            rule_name: r.get(6),
            severity: r.get(7),
            safe_fingerprint: r.get(8),
            status: r.get(9),
            created_at_nanos: r.get(10),
            resolved_at_nanos: r.get(11),
        }).collect())
    }

    async fn update_secret_scan_finding_status(&self, id: &str, status: &str, now_nanos: i64) -> Result<(), ForgeError> {
        let query = "UPDATE secret_scan_findings SET status = $1, resolved_at_nanos = $2 WHERE id = $3";
        sqlx::query(query)
            .bind(status)
            .bind(now_nanos)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn upsert_dependency_inventory(&self, items: &[DependencyInventory]) -> Result<(), ForgeError> {
        for item in items {
            let query = "INSERT INTO dependency_inventories (id, repo_id, ecosystem, package_name, version, is_direct, created_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (repo_id, ecosystem, package_name, version) DO NOTHING";
            sqlx::query(query)
                .bind(&item.id)
                .bind(&item.repo_id)
                .bind(&item.ecosystem)
                .bind(&item.package_name)
                .bind(&item.version)
                .bind(item.is_direct)
                .bind(item.created_at_nanos)
                .execute(&self.pool)
                .await
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
        }
        Ok(())
    }

    async fn list_dependency_inventory(&self, repo_id: &str) -> Result<Vec<DependencyInventory>, ForgeError> {
        let query = "SELECT id, repo_id, ecosystem, package_name, version, is_direct, created_at_nanos FROM dependency_inventories WHERE repo_id = $1 ORDER BY package_name ASC";
        let rows = sqlx::query(query)
            .bind(repo_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| DependencyInventory {
            id: r.get(0),
            repo_id: r.get(1),
            ecosystem: r.get(2),
            package_name: r.get(3),
            version: r.get(4),
            is_direct: r.get(5),
            created_at_nanos: r.get(6),
        }).collect())
    }

    async fn upsert_security_advisory(&self, advisory: SecurityAdvisory) -> Result<SecurityAdvisory, ForgeError> {
        let query = "INSERT INTO security_advisories (id, advisory_id, ecosystem, package_name, vulnerable_version_range, patched_version, severity, title, description, published_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) ON CONFLICT (advisory_id) DO UPDATE SET title = EXCLUDED.title, description = EXCLUDED.description, vulnerable_version_range = EXCLUDED.vulnerable_version_range, patched_version = EXCLUDED.patched_version, severity = EXCLUDED.severity";
        sqlx::query(query)
            .bind(&advisory.id)
            .bind(&advisory.advisory_id)
            .bind(&advisory.ecosystem)
            .bind(&advisory.package_name)
            .bind(&advisory.vulnerable_version_range)
            .bind(&advisory.patched_version)
            .bind(&advisory.severity)
            .bind(&advisory.title)
            .bind(&advisory.description)
            .bind(advisory.published_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(advisory)
    }

    async fn list_security_advisories(&self, ecosystem: &str) -> Result<Vec<SecurityAdvisory>, ForgeError> {
        let query = "SELECT id, advisory_id, ecosystem, package_name, vulnerable_version_range, patched_version, severity, title, description, published_at_nanos FROM security_advisories WHERE ecosystem = $1 ORDER BY published_at_nanos DESC";
        let rows = sqlx::query(query)
            .bind(ecosystem)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| SecurityAdvisory {
            id: r.get(0),
            advisory_id: r.get(1),
            ecosystem: r.get(2),
            package_name: r.get(3),
            vulnerable_version_range: r.get(4),
            patched_version: r.get(5),
            severity: r.get(6),
            title: r.get(7),
            description: r.get(8),
            published_at_nanos: r.get(9),
        }).collect())
    }

    async fn create_dependency_vulnerability(&self, vuln: DependencyVulnerability) -> Result<DependencyVulnerability, ForgeError> {
        let query = "INSERT INTO dependency_vulnerabilities (id, repo_id, dependency_id, advisory_id, package_name, affected_version, severity, patched_version, status, created_at_nanos, resolved_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) ON CONFLICT (repo_id, dependency_id, advisory_id) DO NOTHING";
        sqlx::query(query)
            .bind(&vuln.id)
            .bind(&vuln.repo_id)
            .bind(&vuln.dependency_id)
            .bind(&vuln.advisory_id)
            .bind(&vuln.package_name)
            .bind(&vuln.affected_version)
            .bind(&vuln.severity)
            .bind(&vuln.patched_version)
            .bind(&vuln.status)
            .bind(vuln.created_at_nanos)
            .bind(vuln.resolved_at_nanos)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(vuln)
    }

    async fn list_dependency_vulnerabilities(&self, repo_id: &str, status_filter: Option<&str>) -> Result<Vec<DependencyVulnerability>, ForgeError> {
        let query = if let Some(st) = status_filter {
            sqlx::query("SELECT id, repo_id, dependency_id, advisory_id, package_name, affected_version, severity, patched_version, status, created_at_nanos, resolved_at_nanos FROM dependency_vulnerabilities WHERE repo_id = $1 AND status = $2 ORDER BY created_at_nanos DESC")
                .bind(repo_id)
                .bind(st)
        } else {
            sqlx::query("SELECT id, repo_id, dependency_id, advisory_id, package_name, affected_version, severity, patched_version, status, created_at_nanos, resolved_at_nanos FROM dependency_vulnerabilities WHERE repo_id = $1 ORDER BY created_at_nanos DESC")
                .bind(repo_id)
        };
        let rows = query.fetch_all(&self.pool).await.map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| DependencyVulnerability {
            id: r.get(0),
            repo_id: r.get(1),
            dependency_id: r.get(2),
            advisory_id: r.get(3),
            package_name: r.get(4),
            affected_version: r.get(5),
            severity: r.get(6),
            patched_version: r.get(7),
            status: r.get(8),
            created_at_nanos: r.get(9),
            resolved_at_nanos: r.get(10),
        }).collect())
    }

    async fn update_dependency_vulnerability_status(&self, id: &str, status: &str, now_nanos: i64) -> Result<(), ForgeError> {
        let query = "UPDATE dependency_vulnerabilities SET status = $1, resolved_at_nanos = $2 WHERE id = $3";
        sqlx::query(query)
            .bind(status)
            .bind(now_nanos)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(())
    }
}
