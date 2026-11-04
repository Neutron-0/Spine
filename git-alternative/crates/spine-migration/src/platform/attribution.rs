use super::models::PlatformUser;
use std::collections::HashMap;

/// Policy for mapping external platform users to native Spine accounts.
#[derive(Debug, Clone)]
pub struct AttributionPolicy {
    /// Explicit mapping: external username or email -> Spine user ID or username
    pub explicit_map: HashMap<String, String>,
    /// Fallback user (typically the migration operator / admin) if unmapped
    pub fallback_spine_user: String,
    /// Whether to generate placeholder / tombstone accounts for unmapped external users
    pub create_tombstone_placeholders: bool,
}

impl Default for AttributionPolicy {
    fn default() -> Self {
        Self {
            explicit_map: HashMap::new(),
            fallback_spine_user: "admin".to_string(),
            create_tombstone_placeholders: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAttribution {
    pub spine_username: String,
    pub is_tombstone: bool,
    pub original_external_user: String,
}

impl AttributionPolicy {
    /// Resolves an external user according to the attribution policy.
    pub fn resolve_user(&self, user: &PlatformUser) -> ResolvedAttribution {
        // 1. Check explicit map by email
        if let Some(ref email) = user.email {
            if let Some(mapped) = self.explicit_map.get(email) {
                return ResolvedAttribution {
                    spine_username: mapped.clone(),
                    is_tombstone: false,
                    original_external_user: user.username.clone(),
                };
            }
        }

        // 2. Check explicit map by username
        if let Some(mapped) = self.explicit_map.get(&user.username) {
            return ResolvedAttribution {
                spine_username: mapped.clone(),
                is_tombstone: false,
                original_external_user: user.username.clone(),
            };
        }

        // 3. Fallback or placeholder tombstone
        if self.create_tombstone_placeholders {
            let tombstone_name = format!("{}-migrated", sanitize_username(&user.username));
            ResolvedAttribution {
                spine_username: tombstone_name,
                is_tombstone: true,
                original_external_user: user.username.clone(),
            }
        } else {
            ResolvedAttribution {
                spine_username: self.fallback_spine_user.clone(),
                is_tombstone: false,
                original_external_user: user.username.clone(),
            }
        }
    }
}

fn sanitize_username(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}
