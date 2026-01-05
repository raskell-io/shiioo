use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Unique identifier for an audit log entry
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AuditId(pub String);

impl AuditId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

/// Audit event severity level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuditSeverity {
    Info,
    Warning,
    Critical,
}

/// Audit event category
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuditCategory {
    Authentication,
    Authorization,
    DataAccess,
    DataModification,
    ConfigChange,
    SecretAccess,
    WorkflowExecution,
    SystemEvent,
    SecurityEvent,
    ComplianceEvent,
}

/// Audit event action types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuditAction {
    // Authentication events
    UserLogin { user_id: String, ip_address: String },
    UserLogout { user_id: String },
    LoginFailed { user_id: String, reason: String },

    // Authorization events
    PermissionGranted { user_id: String, permission: String, resource: String },
    PermissionDenied { user_id: String, permission: String, resource: String },
    UnauthorizedAccess { user_id: String, resource: String },
    RoleAssigned { user_id: String, role: String },
    RoleRevoked { user_id: String, role: String },

    // Data access
    SecretAccessed { secret_id: String, user_id: String },
    SecretCreated { secret_id: String, secret_type: String },
    SecretRotated { secret_id: String, version: u32 },
    SecretDeleted { secret_id: String },
    DataAccessed { resource_type: String, resource_id: String },
    DataDeleted { resource_type: String, resource_id: String },

    // Data modification
    WorkflowCreated { workflow_id: String, created_by: String },
    WorkflowExecuted { run_id: String, workflow_id: String },
    WorkflowFailed { run_id: String, error: String },

    // Configuration changes
    ConfigChanged { change_id: String, change_type: String, approved_by: Option<String> },
    TenantCreated { tenant_id: String, created_by: String },
    TenantSuspended { tenant_id: String, suspended_by: String },

    // System events
    SystemStartup,
    NodeRegistered { node_id: String, address: String },
    NodeRemoved { node_id: String },
    LeaderElected { node_id: String },

    // Security events
    SecurityScanStarted { scan_id: String, scan_type: String },
    SecurityScanCompleted { scan_id: String, findings_count: usize },
    SecurityIncident { incident_id: String, description: String },
    VulnerabilityDetected { vulnerability_id: String, severity: String },

    // Compliance events
    ComplianceCheckStarted { check_id: String, framework: String },
    ComplianceCheckCompleted { check_id: String, passed: bool },
    DataRetentionPolicyApplied { policy_id: String, records_deleted: usize },
}

/// Tamper-proof audit log entry with chain verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: AuditId,
    pub timestamp: DateTime<Utc>,
    pub category: AuditCategory,
    pub severity: AuditSeverity,
    pub action: AuditAction,
    pub user_id: Option<String>,
    pub tenant_id: Option<String>,
    pub ip_address: Option<String>,
    pub metadata: HashMap<String, String>,
    /// Hash of previous entry (blockchain-style)
    pub previous_hash: Option<String>,
    /// Hash of this entry's content
    pub entry_hash: String,
}

impl AuditEntry {
    /// Create a new audit entry
    pub fn new(
        category: AuditCategory,
        severity: AuditSeverity,
        action: AuditAction,
        user_id: Option<String>,
        tenant_id: Option<String>,
        ip_address: Option<String>,
        metadata: HashMap<String, String>,
        previous_hash: Option<String>,
    ) -> Self {
        let id = AuditId::generate();
        let timestamp = Utc::now();

        // Calculate hash of entry content
        let entry_hash = Self::calculate_hash(
            &id,
            &timestamp,
            &category,
            &severity,
            &action,
            &user_id,
            &tenant_id,
            &previous_hash,
        );

        Self {
            id,
            timestamp,
            category,
            severity,
            action,
            user_id,
            tenant_id,
            ip_address,
            metadata,
            previous_hash,
            entry_hash,
        }
    }

    /// Calculate hash of entry content for tamper detection
    fn calculate_hash(
        id: &AuditId,
        timestamp: &DateTime<Utc>,
        category: &AuditCategory,
        severity: &AuditSeverity,
        action: &AuditAction,
        user_id: &Option<String>,
        tenant_id: &Option<String>,
        previous_hash: &Option<String>,
    ) -> String {
        let mut hasher = Sha256::new();

        hasher.update(id.0.as_bytes());
        hasher.update(timestamp.to_rfc3339().as_bytes());
        hasher.update(format!("{:?}", category).as_bytes());
        hasher.update(format!("{:?}", severity).as_bytes());
        hasher.update(serde_json::to_string(action).unwrap_or_default().as_bytes());

        if let Some(uid) = user_id {
            hasher.update(uid.as_bytes());
        }
        if let Some(tid) = tenant_id {
            hasher.update(tid.as_bytes());
        }
        if let Some(prev) = previous_hash {
            hasher.update(prev.as_bytes());
        }

        hex::encode(hasher.finalize())
    }

    /// Verify entry integrity
    pub fn verify_hash(&self) -> bool {
        let calculated_hash = Self::calculate_hash(
            &self.id,
            &self.timestamp,
            &self.category,
            &self.severity,
            &self.action,
            &self.user_id,
            &self.tenant_id,
            &self.previous_hash,
        );

        calculated_hash == self.entry_hash
    }
}

/// Tamper-proof audit log manager
pub struct AuditLog {
    entries: Arc<Mutex<Vec<AuditEntry>>>,
    last_hash: Arc<Mutex<Option<String>>>,
}

impl AuditLog {
    /// Create a new audit log
    pub fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(Vec::new())),
            last_hash: Arc::new(Mutex::new(None)),
        }
    }

    /// Record an audit event
    pub fn record(
        &self,
        category: AuditCategory,
        severity: AuditSeverity,
        action: AuditAction,
        user_id: Option<String>,
        tenant_id: Option<String>,
        ip_address: Option<String>,
        metadata: HashMap<String, String>,
    ) -> AuditEntry {
        let mut entries = self.entries.lock().unwrap();
        let mut last_hash = self.last_hash.lock().unwrap();

        let entry = AuditEntry::new(
            category,
            severity,
            action,
            user_id,
            tenant_id,
            ip_address,
            metadata,
            last_hash.clone(),
        );

        // Update last hash
        *last_hash = Some(entry.entry_hash.clone());

        entries.push(entry.clone());

        tracing::info!(
            audit_id = %entry.id.0,
            category = ?entry.category,
            severity = ?entry.severity,
            "Audit event recorded"
        );

        entry
    }

    /// Log an audit event (convenience method, same as record)
    pub fn log(
        &self,
        category: AuditCategory,
        severity: AuditSeverity,
        action: AuditAction,
        user_id: Option<String>,
        tenant_id: Option<String>,
        ip_address: Option<String>,
    ) -> AuditEntry {
        self.record(category, severity, action, user_id, tenant_id, ip_address, HashMap::new())
    }

    /// Get all audit entries
    pub fn list_entries(&self) -> Vec<AuditEntry> {
        self.entries.lock().unwrap().clone()
    }

    /// Get all audit entries (alias for list_entries)
    pub fn list_all(&self) -> Vec<AuditEntry> {
        self.list_entries()
    }

    /// Get audit entries by category
    pub fn list_by_category(&self, category: AuditCategory) -> Vec<AuditEntry> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.category == category)
            .cloned()
            .collect()
    }

    /// Get audit entries by severity
    pub fn list_by_severity(&self, severity: AuditSeverity) -> Vec<AuditEntry> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.severity == severity)
            .cloned()
            .collect()
    }

    /// Get audit entries by user
    pub fn list_by_user(&self, user_id: &str) -> Vec<AuditEntry> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.user_id.as_ref().map(|u| u.as_str()) == Some(user_id))
            .cloned()
            .collect()
    }

    /// Get audit entries by tenant
    pub fn list_by_tenant(&self, tenant_id: &str) -> Vec<AuditEntry> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.tenant_id.as_ref().map(|t| t.as_str()) == Some(tenant_id))
            .cloned()
            .collect()
    }

    /// Get audit entries in time range
    pub fn list_by_time_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Vec<AuditEntry> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.timestamp >= start && e.timestamp <= end)
            .cloned()
            .collect()
    }

    /// Filter entries by category (alias for list_by_category)
    pub fn filter_by_category(&self, category: AuditCategory) -> Vec<AuditEntry> {
        self.list_by_category(category)
    }

    /// Filter entries by time range (alias for list_by_time_range)
    pub fn filter_by_time_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Vec<AuditEntry> {
        self.list_by_time_range(start, end)
    }

    /// Verify chain integrity (returns bool for convenience)
    pub fn verify_chain(&self) -> bool {
        self.verify_chain_detailed().is_ok()
    }

    /// Verify chain integrity with detailed error messages
    pub fn verify_chain_detailed(&self) -> Result<(), Vec<String>> {
        let entries = self.entries.lock().unwrap();
        let mut errors = Vec::new();

        for (i, entry) in entries.iter().enumerate() {
            // Verify entry hash
            if !entry.verify_hash() {
                errors.push(format!("Entry {} hash mismatch", entry.id.0));
            }

            // Verify chain link
            if i > 0 {
                let prev_entry = &entries[i - 1];
                if entry.previous_hash.as_ref() != Some(&prev_entry.entry_hash) {
                    errors.push(format!(
                        "Chain broken at entry {} (expected previous hash: {}, got: {:?})",
                        entry.id.0,
                        prev_entry.entry_hash,
                        entry.previous_hash
                    ));
                }
            } else if entry.previous_hash.is_some() {
                errors.push(format!(
                    "First entry {} should not have previous hash",
                    entry.id.0
                ));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Get audit statistics
    pub fn get_statistics(&self) -> AuditStatistics {
        let entries = self.entries.lock().unwrap();

        let mut by_category = HashMap::new();
        let mut by_severity = HashMap::new();

        for entry in entries.iter() {
            *by_category.entry(entry.category).or_insert(0) += 1;
            *by_severity.entry(entry.severity).or_insert(0) += 1;
        }

        AuditStatistics {
            total_entries: entries.len(),
            by_category,
            by_severity,
            oldest_entry: entries.first().map(|e| e.timestamp),
            newest_entry: entries.last().map(|e| e.timestamp),
        }
    }
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::new()
    }
}

/// Audit log statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditStatistics {
    pub total_entries: usize,
    pub by_category: HashMap<AuditCategory, usize>,
    pub by_severity: HashMap<AuditSeverity, usize>,
    pub oldest_entry: Option<DateTime<Utc>>,
    pub newest_entry: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_entry_hash() {
        let entry = AuditEntry::new(
            AuditCategory::Authentication,
            AuditSeverity::Info,
            AuditAction::UserLogin {
                user_id: "user1".to_string(),
                ip_address: "127.0.0.1".to_string(),
            },
            Some("user1".to_string()),
            None,
            Some("127.0.0.1".to_string()),
            HashMap::new(),
            None,
        );

        assert!(entry.verify_hash());
    }

    #[test]
    fn test_audit_log_record() {
        let log = AuditLog::new();

        let entry = log.record(
            AuditCategory::Authentication,
            AuditSeverity::Info,
            AuditAction::UserLogin {
                user_id: "user1".to_string(),
                ip_address: "127.0.0.1".to_string(),
            },
            Some("user1".to_string()),
            None,
            Some("127.0.0.1".to_string()),
            HashMap::new(),
        );

        assert!(entry.verify_hash());
        assert_eq!(log.list_entries().len(), 1);
    }

    #[test]
    fn test_audit_chain_integrity() {
        let log = AuditLog::new();

        // Record multiple entries
        for i in 0..5 {
            log.record(
                AuditCategory::Authentication,
                AuditSeverity::Info,
                AuditAction::UserLogin {
                    user_id: format!("user{}", i),
                    ip_address: "127.0.0.1".to_string(),
                },
                Some(format!("user{}", i)),
                None,
                Some("127.0.0.1".to_string()),
                HashMap::new(),
            );
        }

        // Verify chain
        assert!(log.verify_chain());
    }

    #[test]
    fn test_audit_filter_by_category() {
        let log = AuditLog::new();

        log.record(
            AuditCategory::Authentication,
            AuditSeverity::Info,
            AuditAction::UserLogin {
                user_id: "user1".to_string(),
                ip_address: "127.0.0.1".to_string(),
            },
            Some("user1".to_string()),
            None,
            None,
            HashMap::new(),
        );

        log.record(
            AuditCategory::SecretAccess,
            AuditSeverity::Warning,
            AuditAction::SecretAccessed {
                secret_id: "secret1".to_string(),
                user_id: "user1".to_string(),
            },
            Some("user1".to_string()),
            None,
            None,
            HashMap::new(),
        );

        let auth_entries = log.list_by_category(AuditCategory::Authentication);
        assert_eq!(auth_entries.len(), 1);

        let secret_entries = log.list_by_category(AuditCategory::SecretAccess);
        assert_eq!(secret_entries.len(), 1);
    }

    #[test]
    fn test_audit_filter_by_user() {
        let log = AuditLog::new();

        log.record(
            AuditCategory::Authentication,
            AuditSeverity::Info,
            AuditAction::UserLogin {
                user_id: "user1".to_string(),
                ip_address: "127.0.0.1".to_string(),
            },
            Some("user1".to_string()),
            None,
            None,
            HashMap::new(),
        );

        log.record(
            AuditCategory::Authentication,
            AuditSeverity::Info,
            AuditAction::UserLogin {
                user_id: "user2".to_string(),
                ip_address: "127.0.0.1".to_string(),
            },
            Some("user2".to_string()),
            None,
            None,
            HashMap::new(),
        );

        let user1_entries = log.list_by_user("user1");
        assert_eq!(user1_entries.len(), 1);
        assert_eq!(user1_entries[0].user_id, Some("user1".to_string()));
    }

    #[test]
    fn test_audit_statistics() {
        let log = AuditLog::new();

        log.record(
            AuditCategory::Authentication,
            AuditSeverity::Info,
            AuditAction::UserLogin {
                user_id: "user1".to_string(),
                ip_address: "127.0.0.1".to_string(),
            },
            Some("user1".to_string()),
            None,
            None,
            HashMap::new(),
        );

        log.record(
            AuditCategory::Authentication,
            AuditSeverity::Critical,
            AuditAction::LoginFailed {
                user_id: "user2".to_string(),
                reason: "Invalid password".to_string(),
            },
            None,
            None,
            Some("127.0.0.1".to_string()),
            HashMap::new(),
        );

        let stats = log.get_statistics();
        assert_eq!(stats.total_entries, 2);
        assert_eq!(stats.by_category.get(&AuditCategory::Authentication), Some(&2));
        assert_eq!(stats.by_severity.get(&AuditSeverity::Info), Some(&1));
        assert_eq!(stats.by_severity.get(&AuditSeverity::Critical), Some(&1));
    }

    #[test]
    fn test_tamper_detection() {
        let log = AuditLog::new();

        log.record(
            AuditCategory::Authentication,
            AuditSeverity::Info,
            AuditAction::UserLogin {
                user_id: "user1".to_string(),
                ip_address: "127.0.0.1".to_string(),
            },
            Some("user1".to_string()),
            None,
            None,
            HashMap::new(),
        );

        log.record(
            AuditCategory::Authentication,
            AuditSeverity::Info,
            AuditAction::UserLogin {
                user_id: "user2".to_string(),
                ip_address: "127.0.0.1".to_string(),
            },
            Some("user2".to_string()),
            None,
            None,
            HashMap::new(),
        );

        // Tamper with an entry
        {
            let mut entries = log.entries.lock().unwrap();
            entries[0].user_id = Some("tampered".to_string());
        }

        // Verification should fail
        assert!(!log.verify_chain());
    }
}

