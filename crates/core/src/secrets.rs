use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Unique identifier for a secret
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SecretId(pub String);

impl SecretId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

/// Secret type classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecretType {
    /// API key or access token
    ApiKey,
    /// Database password
    DatabasePassword,
    /// Private key (RSA, Ed25519, etc.)
    PrivateKey,
    /// OAuth credentials
    OAuthCredentials,
    /// Generic secret
    Generic,
}

/// Secret rotation policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationPolicy {
    /// Enable automatic rotation
    pub enabled: bool,
    /// Rotation interval in days
    pub rotation_interval_days: u32,
    /// Grace period for old secrets (days)
    pub grace_period_days: u32,
    /// Notify before rotation (days)
    pub notify_before_days: u32,
}

impl Default for RotationPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            rotation_interval_days: 90,
            grace_period_days: 7,
            notify_before_days: 7,
        }
    }
}

/// Secret metadata and encrypted value
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Secret {
    pub id: SecretId,
    pub name: String,
    pub description: String,
    pub secret_type: SecretType,
    /// Encrypted secret value (base64-encoded)
    pub encrypted_value: String,
    /// Hash of the plaintext value (for verification)
    pub value_hash: String,
    /// Current version number
    pub version: u32,
    /// Rotation policy
    pub rotation_policy: RotationPolicy,
    /// Tags for organization
    pub tags: HashMap<String, String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_rotated_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Secret version history entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretVersion {
    pub secret_id: SecretId,
    pub version: u32,
    pub encrypted_value: String,
    pub value_hash: String,
    pub created_at: DateTime<Utc>,
    pub deprecated_at: Option<DateTime<Utc>>,
}

/// Simple encryption/decryption using XOR cipher
/// NOTE: This is for demonstration. Production should use proper encryption (AES-GCM, etc.)
pub struct SecretEncryption {
    key: Vec<u8>,
}

impl SecretEncryption {
    /// Create a new encryption instance with a key
    pub fn new(key: &[u8]) -> Self {
        Self {
            key: key.to_vec(),
        }
    }

    /// Encrypt plaintext value
    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        let plaintext_bytes = plaintext.as_bytes();
        let mut encrypted = Vec::with_capacity(plaintext_bytes.len());

        for (i, byte) in plaintext_bytes.iter().enumerate() {
            let key_byte = self.key[i % self.key.len()];
            encrypted.push(byte ^ key_byte);
        }

        Ok(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &encrypted,
        ))
    }

    /// Decrypt encrypted value
    pub fn decrypt(&self, encrypted: &str) -> Result<String> {
        let encrypted_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            encrypted,
        )
        .context("Failed to decode base64")?;

        let mut decrypted = Vec::with_capacity(encrypted_bytes.len());

        for (i, byte) in encrypted_bytes.iter().enumerate() {
            let key_byte = self.key[i % self.key.len()];
            decrypted.push(byte ^ key_byte);
        }

        String::from_utf8(decrypted).context("Failed to decode UTF-8")
    }

    /// Hash a value for verification
    pub fn hash(value: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(value.as_bytes());
        hex::encode(hasher.finalize())
    }
}

/// Secret manager for storing and retrieving encrypted secrets
pub struct SecretManager {
    secrets: Arc<Mutex<HashMap<SecretId, Secret>>>,
    versions: Arc<Mutex<HashMap<SecretId, Vec<SecretVersion>>>>,
    encryption: Arc<SecretEncryption>,
}

impl SecretManager {
    /// Create a new secret manager with encryption key
    pub fn new(encryption_key: &[u8]) -> Self {
        Self {
            secrets: Arc::new(Mutex::new(HashMap::new())),
            versions: Arc::new(Mutex::new(HashMap::new())),
            encryption: Arc::new(SecretEncryption::new(encryption_key)),
        }
    }

    /// Store a new secret
    pub fn create_secret(
        &self,
        name: String,
        description: String,
        secret_type: SecretType,
        value: String,
        rotation_policy: Option<RotationPolicy>,
        tags: HashMap<String, String>,
    ) -> Result<Secret> {
        let encrypted_value = self.encryption.encrypt(&value)?;
        let value_hash = SecretEncryption::hash(&value);

        let secret = Secret {
            id: SecretId::generate(),
            name,
            description,
            secret_type,
            encrypted_value: encrypted_value.clone(),
            value_hash: value_hash.clone(),
            version: 1,
            rotation_policy: rotation_policy.unwrap_or_default(),
            tags,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_rotated_at: None,
            expires_at: None,
        };

        // Store version history
        let version = SecretVersion {
            secret_id: secret.id.clone(),
            version: 1,
            encrypted_value,
            value_hash,
            created_at: secret.created_at,
            deprecated_at: None,
        };

        let mut secrets = self.secrets.lock().unwrap();
        let mut versions = self.versions.lock().unwrap();

        secrets.insert(secret.id.clone(), secret.clone());
        versions.insert(secret.id.clone(), vec![version]);

        tracing::info!("Created secret: {} ({})", secret.name, secret.id.0);

        Ok(secret)
    }

    /// Get secret metadata (without decrypted value)
    pub fn get_secret(&self, secret_id: &SecretId) -> Option<Secret> {
        self.secrets.lock().unwrap().get(secret_id).cloned()
    }

    /// Get decrypted secret value
    pub fn get_secret_value(&self, secret_id: &SecretId) -> Result<String> {
        let secrets = self.secrets.lock().unwrap();
        let secret = secrets
            .get(secret_id)
            .ok_or_else(|| anyhow::anyhow!("Secret not found: {}", secret_id.0))?;

        self.encryption.decrypt(&secret.encrypted_value)
    }

    /// List all secrets (without values)
    pub fn list_secrets(&self) -> Vec<Secret> {
        self.secrets.lock().unwrap().values().cloned().collect()
    }

    /// Update secret value (creates new version)
    pub fn rotate_secret(&self, secret_id: &SecretId, new_value: String) -> Result<Secret> {
        let encrypted_value = self.encryption.encrypt(&new_value)?;
        let value_hash = SecretEncryption::hash(&new_value);

        let mut secrets = self.secrets.lock().unwrap();
        let mut versions = self.versions.lock().unwrap();

        let secret = secrets
            .get_mut(secret_id)
            .ok_or_else(|| anyhow::anyhow!("Secret not found: {}", secret_id.0))?;

        // Deprecate old version
        if let Some(version_history) = versions.get_mut(secret_id) {
            if let Some(last_version) = version_history.last_mut() {
                last_version.deprecated_at = Some(Utc::now());
            }

            // Add new version
            version_history.push(SecretVersion {
                secret_id: secret_id.clone(),
                version: secret.version + 1,
                encrypted_value: encrypted_value.clone(),
                value_hash: value_hash.clone(),
                created_at: Utc::now(),
                deprecated_at: None,
            });
        }

        // Update secret
        secret.encrypted_value = encrypted_value;
        secret.value_hash = value_hash;
        secret.version += 1;
        secret.updated_at = Utc::now();
        secret.last_rotated_at = Some(Utc::now());

        tracing::info!(
            "Rotated secret: {} to version {}",
            secret.name,
            secret.version
        );

        Ok(secret.clone())
    }

    /// Delete a secret
    pub fn delete_secret(&self, secret_id: &SecretId) -> Result<()> {
        let mut secrets = self.secrets.lock().unwrap();
        let mut versions = self.versions.lock().unwrap();

        secrets
            .remove(secret_id)
            .ok_or_else(|| anyhow::anyhow!("Secret not found: {}", secret_id.0))?;

        versions.remove(secret_id);

        tracing::info!("Deleted secret: {}", secret_id.0);

        Ok(())
    }

    /// Get secret version history
    pub fn get_secret_versions(&self, secret_id: &SecretId) -> Vec<SecretVersion> {
        self.versions
            .lock()
            .unwrap()
            .get(secret_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Get a specific version of a secret value
    pub fn get_secret_value_version(
        &self,
        secret_id: &SecretId,
        version: u32,
    ) -> Result<String> {
        let versions = self.versions.lock().unwrap();
        let version_history = versions
            .get(secret_id)
            .ok_or_else(|| anyhow::anyhow!("Secret not found: {}", secret_id.0))?;

        let secret_version = version_history
            .iter()
            .find(|v| v.version == version)
            .ok_or_else(|| {
                anyhow::anyhow!("Secret version {} not found", version)
            })?;

        self.encryption.decrypt(&secret_version.encrypted_value)
    }

    /// Check which secrets need rotation
    pub fn get_secrets_needing_rotation(&self) -> Vec<Secret> {
        let secrets = self.secrets.lock().unwrap();
        let now = Utc::now();

        secrets
            .values()
            .filter(|s| {
                if !s.rotation_policy.enabled {
                    return false;
                }

                let last_rotation = s.last_rotated_at.unwrap_or(s.created_at);
                let rotation_due = last_rotation
                    + Duration::days(s.rotation_policy.rotation_interval_days as i64);

                now >= rotation_due
            })
            .cloned()
            .collect()
    }

    /// Get secrets expiring soon
    pub fn get_expiring_secrets(&self, days: i64) -> Vec<Secret> {
        let secrets = self.secrets.lock().unwrap();
        let threshold = Utc::now() + Duration::days(days);

        secrets
            .values()
            .filter(|s| {
                if let Some(expires_at) = s.expires_at {
                    expires_at <= threshold
                } else {
                    false
                }
            })
            .cloned()
            .collect()
    }

    /// Update secret metadata (name, description, tags, policy)
    pub fn update_secret_metadata(
        &self,
        secret_id: &SecretId,
        name: Option<String>,
        description: Option<String>,
        rotation_policy: Option<RotationPolicy>,
        tags: Option<HashMap<String, String>>,
    ) -> Result<Secret> {
        let mut secrets = self.secrets.lock().unwrap();
        let secret = secrets
            .get_mut(secret_id)
            .ok_or_else(|| anyhow::anyhow!("Secret not found: {}", secret_id.0))?;

        if let Some(name) = name {
            secret.name = name;
        }
        if let Some(description) = description {
            secret.description = description;
        }
        if let Some(rotation_policy) = rotation_policy {
            secret.rotation_policy = rotation_policy;
        }
        if let Some(tags) = tags {
            secret.tags = tags;
        }

        secret.updated_at = Utc::now();

        Ok(secret.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encryption_decrypt() {
        let encryption = SecretEncryption::new(b"test-key-32-bytes-long-for-aes");
        let plaintext = "my-secret-api-key";

        let encrypted = encryption.encrypt(plaintext).unwrap();
        let decrypted = encryption.decrypt(&encrypted).unwrap();

        assert_eq!(plaintext, decrypted);
    }

    #[test]
    fn test_hash_consistency() {
        let hash1 = SecretEncryption::hash("my-secret");
        let hash2 = SecretEncryption::hash("my-secret");
        let hash3 = SecretEncryption::hash("different-secret");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_create_secret() {
        let manager = SecretManager::new(b"test-key-32-bytes-long-for-aes");

        let secret = manager
            .create_secret(
                "API Key".to_string(),
                "Production API key".to_string(),
                SecretType::ApiKey,
                "sk-test-12345".to_string(),
                None,
                HashMap::new(),
            )
            .unwrap();

        assert_eq!(secret.name, "API Key");
        assert_eq!(secret.version, 1);
        assert!(secret.last_rotated_at.is_none());
    }

    #[test]
    fn test_get_secret_value() {
        let manager = SecretManager::new(b"test-key-32-bytes-long-for-aes");

        let secret = manager
            .create_secret(
                "API Key".to_string(),
                "Test key".to_string(),
                SecretType::ApiKey,
                "sk-test-12345".to_string(),
                None,
                HashMap::new(),
            )
            .unwrap();

        let value = manager.get_secret_value(&secret.id).unwrap();
        assert_eq!(value, "sk-test-12345");
    }

    #[test]
    fn test_rotate_secret() {
        let manager = SecretManager::new(b"test-key-32-bytes-long-for-aes");

        let secret = manager
            .create_secret(
                "API Key".to_string(),
                "Test key".to_string(),
                SecretType::ApiKey,
                "sk-test-12345".to_string(),
                None,
                HashMap::new(),
            )
            .unwrap();

        let rotated = manager
            .rotate_secret(&secret.id, "sk-test-67890".to_string())
            .unwrap();

        assert_eq!(rotated.version, 2);
        assert!(rotated.last_rotated_at.is_some());

        let value = manager.get_secret_value(&secret.id).unwrap();
        assert_eq!(value, "sk-test-67890");

        // Old version should still be accessible
        let old_value = manager.get_secret_value_version(&secret.id, 1).unwrap();
        assert_eq!(old_value, "sk-test-12345");
    }

    #[test]
    fn test_delete_secret() {
        let manager = SecretManager::new(b"test-key-32-bytes-long-for-aes");

        let secret = manager
            .create_secret(
                "API Key".to_string(),
                "Test key".to_string(),
                SecretType::ApiKey,
                "sk-test-12345".to_string(),
                None,
                HashMap::new(),
            )
            .unwrap();

        manager.delete_secret(&secret.id).unwrap();

        assert!(manager.get_secret(&secret.id).is_none());
    }

    #[test]
    fn test_list_secrets() {
        let manager = SecretManager::new(b"test-key-32-bytes-long-for-aes");

        manager
            .create_secret(
                "Key 1".to_string(),
                "First key".to_string(),
                SecretType::ApiKey,
                "value1".to_string(),
                None,
                HashMap::new(),
            )
            .unwrap();

        manager
            .create_secret(
                "Key 2".to_string(),
                "Second key".to_string(),
                SecretType::DatabasePassword,
                "value2".to_string(),
                None,
                HashMap::new(),
            )
            .unwrap();

        let secrets = manager.list_secrets();
        assert_eq!(secrets.len(), 2);
    }

    #[test]
    fn test_rotation_policy() {
        let manager = SecretManager::new(b"test-key-32-bytes-long-for-aes");

        let mut policy = RotationPolicy::default();
        policy.enabled = true;
        policy.rotation_interval_days = 1; // 1 day for testing

        let secret = manager
            .create_secret(
                "API Key".to_string(),
                "Test key".to_string(),
                SecretType::ApiKey,
                "sk-test-12345".to_string(),
                Some(policy),
                HashMap::new(),
            )
            .unwrap();

        // Simulate secret created 2 days ago
        let mut secrets = manager.secrets.lock().unwrap();
        if let Some(s) = secrets.get_mut(&secret.id) {
            s.created_at = Utc::now() - Duration::days(2);
        }
        drop(secrets);

        let needing_rotation = manager.get_secrets_needing_rotation();
        assert_eq!(needing_rotation.len(), 1);
        assert_eq!(needing_rotation[0].id, secret.id);
    }

    #[test]
    fn test_version_history() {
        let manager = SecretManager::new(b"test-key-32-bytes-long-for-aes");

        let secret = manager
            .create_secret(
                "API Key".to_string(),
                "Test key".to_string(),
                SecretType::ApiKey,
                "value1".to_string(),
                None,
                HashMap::new(),
            )
            .unwrap();

        manager.rotate_secret(&secret.id, "value2".to_string()).unwrap();
        manager.rotate_secret(&secret.id, "value3".to_string()).unwrap();

        let versions = manager.get_secret_versions(&secret.id);
        assert_eq!(versions.len(), 3);
        assert_eq!(versions[0].version, 1);
        assert_eq!(versions[1].version, 2);
        assert_eq!(versions[2].version, 3);

        // First two versions should be deprecated
        assert!(versions[0].deprecated_at.is_some());
        assert!(versions[1].deprecated_at.is_some());
        assert!(versions[2].deprecated_at.is_none());
    }
}
