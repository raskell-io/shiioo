use crate::audit::{AuditAction, AuditCategory, AuditEntry, AuditId, AuditLog, AuditSeverity};
use crate::rbac::{Action, Permission, RbacManager, Resource};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Compliance frameworks
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ComplianceFramework {
    SOC2,
    GDPR,
    HIPAA,
    ISO27001,
    PCI_DSS,
}

/// Compliance requirement status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComplianceStatus {
    Compliant,
    NonCompliant,
    PartiallyCompliant,
    NotApplicable,
}

/// Compliance requirement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceRequirement {
    pub id: String,
    pub framework: ComplianceFramework,
    pub title: String,
    pub description: String,
    pub category: String,
    pub status: ComplianceStatus,
    pub evidence: Vec<String>,
    pub findings: Vec<String>,
    pub last_checked: Option<DateTime<Utc>>,
}

impl ComplianceRequirement {
    pub fn new(
        id: String,
        framework: ComplianceFramework,
        title: String,
        description: String,
        category: String,
    ) -> Self {
        Self {
            id,
            framework,
            title,
            description,
            category,
            status: ComplianceStatus::NotApplicable,
            evidence: Vec::new(),
            findings: Vec::new(),
            last_checked: None,
        }
    }

    pub fn add_evidence(&mut self, evidence: String) {
        self.evidence.push(evidence);
    }

    pub fn add_finding(&mut self, finding: String) {
        self.findings.push(finding);
    }

    pub fn update_status(&mut self, status: ComplianceStatus) {
        self.status = status;
        self.last_checked = Some(Utc::now());
    }
}

/// Compliance report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceReport {
    pub id: String,
    pub framework: ComplianceFramework,
    pub generated_at: DateTime<Utc>,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub requirements: Vec<ComplianceRequirement>,
    pub summary: ComplianceSummary,
}

/// Compliance summary statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceSummary {
    pub total_requirements: usize,
    pub compliant: usize,
    pub non_compliant: usize,
    pub partially_compliant: usize,
    pub not_applicable: usize,
    pub compliance_percentage: f64,
}

impl ComplianceSummary {
    pub fn from_requirements(requirements: &[ComplianceRequirement]) -> Self {
        let total = requirements.len();
        let compliant = requirements.iter().filter(|r| r.status == ComplianceStatus::Compliant).count();
        let non_compliant = requirements.iter().filter(|r| r.status == ComplianceStatus::NonCompliant).count();
        let partially_compliant = requirements.iter().filter(|r| r.status == ComplianceStatus::PartiallyCompliant).count();
        let not_applicable = requirements.iter().filter(|r| r.status == ComplianceStatus::NotApplicable).count();

        let applicable = total - not_applicable;
        let compliance_percentage = if applicable > 0 {
            (compliant as f64 / applicable as f64) * 100.0
        } else {
            0.0
        };

        Self {
            total_requirements: total,
            compliant,
            non_compliant,
            partially_compliant,
            not_applicable,
            compliance_percentage,
        }
    }
}

/// Compliance checker
pub struct ComplianceChecker {
    audit_log: AuditLog,
    rbac_manager: RbacManager,
}

impl ComplianceChecker {
    pub fn new(audit_log: AuditLog, rbac_manager: RbacManager) -> Self {
        Self {
            audit_log,
            rbac_manager,
        }
    }

    /// Generate compliance report for a framework
    pub fn generate_report(
        &self,
        framework: ComplianceFramework,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
    ) -> ComplianceReport {
        let requirements = match framework {
            ComplianceFramework::SOC2 => self.check_soc2_compliance(period_start, period_end),
            ComplianceFramework::GDPR => self.check_gdpr_compliance(period_start, period_end),
            ComplianceFramework::HIPAA => self.check_hipaa_compliance(period_start, period_end),
            ComplianceFramework::ISO27001 => self.check_iso27001_compliance(period_start, period_end),
            ComplianceFramework::PCI_DSS => self.check_pci_dss_compliance(period_start, period_end),
        };

        let summary = ComplianceSummary::from_requirements(&requirements);

        ComplianceReport {
            id: uuid::Uuid::new_v4().to_string(),
            framework,
            generated_at: Utc::now(),
            period_start,
            period_end,
            requirements,
            summary,
        }
    }

    /// Check SOC2 compliance
    fn check_soc2_compliance(
        &self,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
    ) -> Vec<ComplianceRequirement> {
        let mut requirements = Vec::new();

        // CC6.1: Access controls - Logical and physical access to systems
        let mut cc6_1 = ComplianceRequirement::new(
            "CC6.1".to_string(),
            ComplianceFramework::SOC2,
            "Logical and Physical Access Controls".to_string(),
            "The entity implements logical access security software, infrastructure, and architectures over protected information assets to protect them from security events to meet the entity's objectives.".to_string(),
            "Access Control".to_string(),
        );

        let failed_logins = self.count_events_by_action(
            period_start,
            period_end,
            |action| matches!(action, AuditAction::LoginFailed { .. }),
        );

        if failed_logins > 0 {
            cc6_1.add_evidence(format!("Access control events logged: {} failed login attempts", failed_logins));
        }

        let unauthorized_access = self.count_events_by_action(
            period_start,
            period_end,
            |action| matches!(action, AuditAction::UnauthorizedAccess { .. }),
        );

        if unauthorized_access > 10 {
            cc6_1.add_finding(format!("High number of unauthorized access attempts: {}", unauthorized_access));
            cc6_1.update_status(ComplianceStatus::PartiallyCompliant);
        } else if unauthorized_access > 0 {
            cc6_1.add_evidence(format!("Unauthorized access attempts detected and logged: {}", unauthorized_access));
            cc6_1.update_status(ComplianceStatus::Compliant);
        } else {
            cc6_1.update_status(ComplianceStatus::Compliant);
        }

        requirements.push(cc6_1);

        // CC6.2: Prior to issuing system credentials, the entity registers and authorizes new users
        let mut cc6_2 = ComplianceRequirement::new(
            "CC6.2".to_string(),
            ComplianceFramework::SOC2,
            "User Registration and Authorization".to_string(),
            "Prior to issuing system credentials and granting system access, the entity registers and authorizes new internal and external users whose access is administered by the entity.".to_string(),
            "Access Control".to_string(),
        );

        let role_assignments = self.count_events_by_action(
            period_start,
            period_end,
            |action| matches!(action, AuditAction::RoleAssigned { .. }),
        );

        if role_assignments > 0 {
            cc6_2.add_evidence(format!("Role assignments tracked: {} assignments", role_assignments));
            cc6_2.update_status(ComplianceStatus::Compliant);
        } else {
            cc6_2.update_status(ComplianceStatus::NotApplicable);
        }

        requirements.push(cc6_2);

        // CC7.2: Detection and monitoring of security events
        let mut cc7_2 = ComplianceRequirement::new(
            "CC7.2".to_string(),
            ComplianceFramework::SOC2,
            "Security Event Detection and Monitoring".to_string(),
            "The entity monitors system components and the operation of those components for anomalies that are indicative of malicious acts, natural disasters, and errors affecting the entity's ability to meet its objectives.".to_string(),
            "Monitoring".to_string(),
        );

        let security_events = self.audit_log.filter_by_category(AuditCategory::SecurityEvent);
        if !security_events.is_empty() {
            cc7_2.add_evidence(format!("Security events monitored: {} events", security_events.len()));
            cc7_2.update_status(ComplianceStatus::Compliant);
        } else {
            cc7_2.update_status(ComplianceStatus::Compliant);
        }

        requirements.push(cc7_2);

        // CC7.3: Audit logs are retained and reviewed
        let mut cc7_3 = ComplianceRequirement::new(
            "CC7.3".to_string(),
            ComplianceFramework::SOC2,
            "Audit Log Retention and Review".to_string(),
            "The entity evaluates security events to determine whether they could or have resulted in a failure of the entity to meet its objectives and, if so, takes actions to prevent or address such failures.".to_string(),
            "Monitoring".to_string(),
        );

        let total_events = self.audit_log.list_all().len();
        if total_events > 0 {
            cc7_3.add_evidence(format!("Audit log retention: {} events over period", total_events));

            // Check if chain integrity is maintained
            if self.audit_log.verify_chain() {
                cc7_3.add_evidence("Audit log chain integrity verified (tamper-proof)".to_string());
                cc7_3.update_status(ComplianceStatus::Compliant);
            } else {
                cc7_3.add_finding("Audit log chain integrity check failed - possible tampering".to_string());
                cc7_3.update_status(ComplianceStatus::NonCompliant);
            }
        } else {
            cc7_3.update_status(ComplianceStatus::Compliant);
        }

        requirements.push(cc7_3);

        requirements
    }

    /// Check GDPR compliance
    fn check_gdpr_compliance(
        &self,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
    ) -> Vec<ComplianceRequirement> {
        let mut requirements = Vec::new();

        // Article 5: Principles relating to processing of personal data
        let mut art5 = ComplianceRequirement::new(
            "Article 5".to_string(),
            ComplianceFramework::GDPR,
            "Data Processing Principles".to_string(),
            "Personal data shall be processed lawfully, fairly and in a transparent manner.".to_string(),
            "Data Processing".to_string(),
        );

        let data_access = self.count_events_by_category(
            period_start,
            period_end,
            AuditCategory::DataAccess,
        );

        if data_access > 0 {
            art5.add_evidence(format!("Data access events logged: {} accesses", data_access));
            art5.update_status(ComplianceStatus::Compliant);
        } else {
            art5.update_status(ComplianceStatus::Compliant);
        }

        requirements.push(art5);

        // Article 17: Right to erasure
        let mut art17 = ComplianceRequirement::new(
            "Article 17".to_string(),
            ComplianceFramework::GDPR,
            "Right to Erasure".to_string(),
            "The data subject shall have the right to obtain from the controller the erasure of personal data.".to_string(),
            "Data Subject Rights".to_string(),
        );

        let data_deletions = self.count_events_by_action(
            period_start,
            period_end,
            |action| matches!(action, AuditAction::DataDeleted { .. }),
        );

        if data_deletions > 0 {
            art17.add_evidence(format!("Data deletion requests processed: {}", data_deletions));
            art17.update_status(ComplianceStatus::Compliant);
        } else {
            art17.update_status(ComplianceStatus::NotApplicable);
        }

        requirements.push(art17);

        // Article 30: Records of processing activities
        let mut art30 = ComplianceRequirement::new(
            "Article 30".to_string(),
            ComplianceFramework::GDPR,
            "Records of Processing Activities".to_string(),
            "Each controller shall maintain a record of processing activities under its responsibility.".to_string(),
            "Documentation".to_string(),
        );

        let processing_events = self.count_events_by_category(
            period_start,
            period_end,
            AuditCategory::DataModification,
        );

        if processing_events > 0 {
            art30.add_evidence(format!("Data processing activities logged: {} events", processing_events));
            art30.update_status(ComplianceStatus::Compliant);
        } else {
            art30.update_status(ComplianceStatus::Compliant);
        }

        requirements.push(art30);

        // Article 32: Security of processing
        let mut art32 = ComplianceRequirement::new(
            "Article 32".to_string(),
            ComplianceFramework::GDPR,
            "Security of Processing".to_string(),
            "The controller and processor shall implement appropriate technical and organizational measures to ensure a level of security appropriate to the risk.".to_string(),
            "Security".to_string(),
        );

        let security_events = self.audit_log.filter_by_category(AuditCategory::SecurityEvent);
        let critical_security_events = security_events.iter()
            .filter(|e| e.severity == AuditSeverity::Critical)
            .count();

        if critical_security_events > 5 {
            art32.add_finding(format!("High number of critical security events: {}", critical_security_events));
            art32.update_status(ComplianceStatus::NonCompliant);
        } else if security_events.is_empty() {
            art32.add_evidence("No security events detected".to_string());
            art32.update_status(ComplianceStatus::Compliant);
        } else {
            art32.add_evidence(format!("Security events monitored: {} events, {} critical", security_events.len(), critical_security_events));
            art32.update_status(ComplianceStatus::Compliant);
        }

        requirements.push(art32);

        // Article 33: Breach notification
        let mut art33 = ComplianceRequirement::new(
            "Article 33".to_string(),
            ComplianceFramework::GDPR,
            "Breach Notification".to_string(),
            "In the case of a personal data breach, the controller shall without undue delay notify the supervisory authority.".to_string(),
            "Incident Response".to_string(),
        );

        let breach_notifications = self.count_events_by_action(
            period_start,
            period_end,
            |action| matches!(action, AuditAction::SecurityIncident { .. }),
        );

        if breach_notifications > 0 {
            art33.add_evidence(format!("Security incidents logged: {}", breach_notifications));
            art33.update_status(ComplianceStatus::Compliant);
        } else {
            art33.update_status(ComplianceStatus::Compliant);
        }

        requirements.push(art33);

        requirements
    }

    /// Check HIPAA compliance (placeholder)
    fn check_hipaa_compliance(
        &self,
        _period_start: DateTime<Utc>,
        _period_end: DateTime<Utc>,
    ) -> Vec<ComplianceRequirement> {
        vec![
            ComplianceRequirement::new(
                "HIPAA-1".to_string(),
                ComplianceFramework::HIPAA,
                "Access Control".to_string(),
                "Implement technical policies and procedures for electronic information systems.".to_string(),
                "Administrative Safeguards".to_string(),
            ),
        ]
    }

    /// Check ISO27001 compliance (placeholder)
    fn check_iso27001_compliance(
        &self,
        _period_start: DateTime<Utc>,
        _period_end: DateTime<Utc>,
    ) -> Vec<ComplianceRequirement> {
        vec![
            ComplianceRequirement::new(
                "ISO27001-1".to_string(),
                ComplianceFramework::ISO27001,
                "Information Security Policy".to_string(),
                "A set of policies for information security shall be defined.".to_string(),
                "Policy".to_string(),
            ),
        ]
    }

    /// Check PCI-DSS compliance (placeholder)
    fn check_pci_dss_compliance(
        &self,
        _period_start: DateTime<Utc>,
        _period_end: DateTime<Utc>,
    ) -> Vec<ComplianceRequirement> {
        vec![
            ComplianceRequirement::new(
                "PCI-DSS-1".to_string(),
                ComplianceFramework::PCI_DSS,
                "Install and Maintain Firewall Configuration".to_string(),
                "Install and maintain a firewall configuration to protect cardholder data.".to_string(),
                "Network Security".to_string(),
            ),
        ]
    }

    /// Helper: Count events by category
    fn count_events_by_category(
        &self,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
        category: AuditCategory,
    ) -> usize {
        self.audit_log
            .filter_by_time_range(period_start, period_end)
            .into_iter()
            .filter(|e| e.category == category)
            .count()
    }

    /// Helper: Count events by action predicate
    fn count_events_by_action<F>(
        &self,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
        predicate: F,
    ) -> usize
    where
        F: Fn(&AuditAction) -> bool,
    {
        self.audit_log
            .filter_by_time_range(period_start, period_end)
            .into_iter()
            .filter(|e| predicate(&e.action))
            .count()
    }
}

/// Security scanner for vulnerability detection
pub struct SecurityScanner {
    audit_log: AuditLog,
}

impl SecurityScanner {
    pub fn new(audit_log: AuditLog) -> Self {
        Self { audit_log }
    }

    /// Scan for security vulnerabilities
    pub fn scan(&self) -> SecurityScanReport {
        let mut findings = Vec::new();

        // Check for brute force attacks
        if let Some(finding) = self.detect_brute_force() {
            findings.push(finding);
        }

        // Check for privilege escalation
        if let Some(finding) = self.detect_privilege_escalation() {
            findings.push(finding);
        }

        // Check for data exfiltration
        if let Some(finding) = self.detect_data_exfiltration() {
            findings.push(finding);
        }

        // Check for suspicious secret access
        if let Some(finding) = self.detect_suspicious_secret_access() {
            findings.push(finding);
        }

        let severity = if findings.iter().any(|f| f.severity == SecuritySeverity::Critical) {
            SecuritySeverity::Critical
        } else if findings.iter().any(|f| f.severity == SecuritySeverity::High) {
            SecuritySeverity::High
        } else if findings.iter().any(|f| f.severity == SecuritySeverity::Medium) {
            SecuritySeverity::Medium
        } else {
            SecuritySeverity::Low
        };

        SecurityScanReport {
            scan_id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            findings,
            overall_severity: severity,
        }
    }

    /// Detect brute force attacks
    fn detect_brute_force(&self) -> Option<SecurityFinding> {
        let recent_time = Utc::now() - Duration::hours(1);
        let failed_logins = self.audit_log
            .filter_by_time_range(recent_time, Utc::now())
            .into_iter()
            .filter(|e| matches!(e.action, AuditAction::LoginFailed { .. }))
            .count();

        if failed_logins > 10 {
            Some(SecurityFinding {
                id: uuid::Uuid::new_v4().to_string(),
                title: "Possible Brute Force Attack".to_string(),
                description: format!("Detected {} failed login attempts in the last hour", failed_logins),
                severity: SecuritySeverity::High,
                recommendation: "Implement rate limiting and account lockout policies".to_string(),
            })
        } else {
            None
        }
    }

    /// Detect privilege escalation attempts
    fn detect_privilege_escalation(&self) -> Option<SecurityFinding> {
        let recent_time = Utc::now() - Duration::hours(24);
        let unauthorized_access = self.audit_log
            .filter_by_time_range(recent_time, Utc::now())
            .into_iter()
            .filter(|e| matches!(e.action, AuditAction::UnauthorizedAccess { .. }))
            .count();

        if unauthorized_access > 5 {
            Some(SecurityFinding {
                id: uuid::Uuid::new_v4().to_string(),
                title: "Possible Privilege Escalation".to_string(),
                description: format!("Detected {} unauthorized access attempts in the last 24 hours", unauthorized_access),
                severity: SecuritySeverity::Critical,
                recommendation: "Review user permissions and implement stricter access controls".to_string(),
            })
        } else {
            None
        }
    }

    /// Detect data exfiltration
    fn detect_data_exfiltration(&self) -> Option<SecurityFinding> {
        let recent_time = Utc::now() - Duration::hours(1);
        let data_reads = self.audit_log
            .filter_by_time_range(recent_time, Utc::now())
            .into_iter()
            .filter(|e| e.category == AuditCategory::DataAccess)
            .count();

        if data_reads > 100 {
            Some(SecurityFinding {
                id: uuid::Uuid::new_v4().to_string(),
                title: "Possible Data Exfiltration".to_string(),
                description: format!("Detected {} data access events in the last hour", data_reads),
                severity: SecuritySeverity::High,
                recommendation: "Investigate data access patterns and implement data loss prevention controls".to_string(),
            })
        } else {
            None
        }
    }

    /// Detect suspicious secret access
    fn detect_suspicious_secret_access(&self) -> Option<SecurityFinding> {
        let recent_time = Utc::now() - Duration::minutes(30);
        let secret_access = self.audit_log
            .filter_by_time_range(recent_time, Utc::now())
            .into_iter()
            .filter(|e| matches!(e.action, AuditAction::SecretAccessed { .. }))
            .count();

        if secret_access > 20 {
            Some(SecurityFinding {
                id: uuid::Uuid::new_v4().to_string(),
                title: "Suspicious Secret Access Pattern".to_string(),
                description: format!("Detected {} secret access events in the last 30 minutes", secret_access),
                severity: SecuritySeverity::Medium,
                recommendation: "Review secret access logs and rotate potentially compromised secrets".to_string(),
            })
        } else {
            None
        }
    }
}

/// Security scan report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityScanReport {
    pub scan_id: String,
    pub timestamp: DateTime<Utc>,
    pub findings: Vec<SecurityFinding>,
    pub overall_severity: SecuritySeverity,
}

/// Security finding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityFinding {
    pub id: String,
    pub title: String,
    pub description: String,
    pub severity: SecuritySeverity,
    pub recommendation: String,
}

/// Security severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecuritySeverity {
    Low,
    Medium,
    High,
    Critical,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{AuditAction, AuditCategory, AuditLog, AuditSeverity};

    #[test]
    fn test_compliance_summary() {
        let mut reqs = Vec::new();
        reqs.push({
            let mut r = ComplianceRequirement::new(
                "1".to_string(),
                ComplianceFramework::SOC2,
                "Test 1".to_string(),
                "Desc".to_string(),
                "Cat".to_string(),
            );
            r.update_status(ComplianceStatus::Compliant);
            r
        });
        reqs.push({
            let mut r = ComplianceRequirement::new(
                "2".to_string(),
                ComplianceFramework::SOC2,
                "Test 2".to_string(),
                "Desc".to_string(),
                "Cat".to_string(),
            );
            r.update_status(ComplianceStatus::NonCompliant);
            r
        });

        let summary = ComplianceSummary::from_requirements(&reqs);
        assert_eq!(summary.total_requirements, 2);
        assert_eq!(summary.compliant, 1);
        assert_eq!(summary.non_compliant, 1);
        assert_eq!(summary.compliance_percentage, 50.0);
    }

    #[test]
    fn test_soc2_compliance_check() {
        let audit_log = AuditLog::new();
        let rbac_manager = RbacManager::new();

        // Add some audit events
        audit_log.log(
            AuditCategory::Authentication,
            AuditSeverity::Warning,
            AuditAction::LoginFailed {
                user_id: "user1".to_string(),
                reason: "Invalid password".to_string(),
            },
            Some("user1".to_string()),
            None,
            None,
        );

        let checker = ComplianceChecker::new(audit_log, rbac_manager);

        let report = checker.generate_report(
            ComplianceFramework::SOC2,
            Utc::now() - Duration::days(30),
            Utc::now(),
        );

        assert_eq!(report.framework, ComplianceFramework::SOC2);
        assert!(!report.requirements.is_empty());
        assert!(report.summary.total_requirements > 0);
    }

    #[test]
    fn test_gdpr_compliance_check() {
        let audit_log = AuditLog::new();
        let rbac_manager = RbacManager::new();

        // Add some data access events
        audit_log.log(
            AuditCategory::DataAccess,
            AuditSeverity::Info,
            AuditAction::DataAccessed {
                resource_type: "user_profile".to_string(),
                resource_id: "profile123".to_string(),
            },
            Some("user1".to_string()),
            None,
            None,
        );

        let checker = ComplianceChecker::new(audit_log, rbac_manager);

        let report = checker.generate_report(
            ComplianceFramework::GDPR,
            Utc::now() - Duration::days(30),
            Utc::now(),
        );

        assert_eq!(report.framework, ComplianceFramework::GDPR);
        assert!(!report.requirements.is_empty());
    }

    #[test]
    fn test_security_scanner_brute_force_detection() {
        let audit_log = AuditLog::new();

        // Simulate brute force attack
        for i in 0..15 {
            audit_log.log(
                AuditCategory::Authentication,
                AuditSeverity::Warning,
                AuditAction::LoginFailed {
                    user_id: format!("user{}", i),
                    reason: "Invalid password".to_string(),
                },
                Some(format!("user{}", i)),
                None,
                None,
            );
        }

        let scanner = SecurityScanner::new(audit_log);
        let report = scanner.scan();

        assert!(!report.findings.is_empty());
        assert!(report.findings.iter().any(|f| f.title.contains("Brute Force")));
    }

    #[test]
    fn test_security_scanner_privilege_escalation() {
        let audit_log = AuditLog::new();

        // Simulate privilege escalation attempts
        for i in 0..10 {
            audit_log.log(
                AuditCategory::Authorization,
                AuditSeverity::Warning,
                AuditAction::UnauthorizedAccess {
                    user_id: "user1".to_string(),
                    resource: "admin_panel".to_string(),
                },
                Some("user1".to_string()),
                None,
                None,
            );
        }

        let scanner = SecurityScanner::new(audit_log);
        let report = scanner.scan();

        assert!(!report.findings.is_empty());
        assert!(report.findings.iter().any(|f| f.title.contains("Privilege Escalation")));
        assert_eq!(report.overall_severity, SecuritySeverity::Critical);
    }

    #[test]
    fn test_audit_chain_verification_in_compliance() {
        let audit_log = AuditLog::new();
        let rbac_manager = RbacManager::new();

        // Add some events to create a chain
        for i in 0..5 {
            audit_log.log(
                AuditCategory::SystemEvent,
                AuditSeverity::Info,
                AuditAction::SystemStartup,
                None,
                None,
                None,
            );
        }

        let checker = ComplianceChecker::new(audit_log, rbac_manager);

        let report = checker.generate_report(
            ComplianceFramework::SOC2,
            Utc::now() - Duration::days(30),
            Utc::now(),
        );

        // Find CC7.3 requirement (audit log retention)
        let cc7_3 = report.requirements.iter().find(|r| r.id == "CC7.3");
        assert!(cc7_3.is_some());

        let cc7_3 = cc7_3.unwrap();
        assert_eq!(cc7_3.status, ComplianceStatus::Compliant);
        assert!(cc7_3.evidence.iter().any(|e| e.contains("chain integrity verified")));
    }
}
