use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// Permission resource types
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Resource {
    Workflow,
    Secret,
    Tenant,
    Cluster,
    Role,
    Policy,
    Approval,
    Routine,
    Organization,
    Template,
    AuditLog,
    All,
}

/// Permission action types
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    Create,
    Read,
    Update,
    Delete,
    Execute,
    Approve,
    Audit,
    All,
}

/// Fine-grained permission
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Permission {
    pub resource: Resource,
    pub action: Action,
    /// Optional resource ID for instance-level permissions
    pub resource_id: Option<String>,
}

impl Permission {
    pub fn new(resource: Resource, action: Action) -> Self {
        Self {
            resource,
            action,
            resource_id: None,
        }
    }

    pub fn with_resource_id(resource: Resource, action: Action, resource_id: String) -> Self {
        Self {
            resource,
            action,
            resource_id: Some(resource_id),
        }
    }

    /// Check if this permission matches another (considering wildcards)
    pub fn matches(&self, other: &Permission) -> bool {
        // Check resource match
        let resource_match = self.resource == Resource::All
            || self.resource == other.resource
            || other.resource == Resource::All;

        // Check action match
        let action_match = self.action == Action::All
            || self.action == other.action
            || other.action == Action::All;

        // Check resource ID match
        let resource_id_match = self.resource_id.is_none()
            || self.resource_id == other.resource_id;

        resource_match && action_match && resource_id_match
    }
}

/// RBAC role with permissions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbacRole {
    pub id: String,
    pub name: String,
    pub description: String,
    pub permissions: HashSet<Permission>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl RbacRole {
    pub fn new(id: String, name: String, description: String) -> Self {
        Self {
            id,
            name,
            description,
            permissions: HashSet::new(),
            created_at: chrono::Utc::now(),
        }
    }

    pub fn add_permission(&mut self, permission: Permission) {
        self.permissions.insert(permission);
    }

    pub fn remove_permission(&mut self, permission: &Permission) -> bool {
        self.permissions.remove(permission)
    }

    pub fn has_permission(&self, permission: &Permission) -> bool {
        self.permissions.iter().any(|p| p.matches(permission))
    }
}

/// User with role assignments
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbacUser {
    pub id: String,
    pub username: String,
    pub email: String,
    pub roles: HashSet<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl RbacUser {
    pub fn new(id: String, username: String, email: String) -> Self {
        Self {
            id,
            username,
            email,
            roles: HashSet::new(),
            created_at: chrono::Utc::now(),
        }
    }

    pub fn assign_role(&mut self, role_id: String) {
        self.roles.insert(role_id);
    }

    pub fn revoke_role(&mut self, role_id: &str) -> bool {
        self.roles.remove(role_id)
    }
}

/// RBAC manager
pub struct RbacManager {
    roles: Arc<Mutex<HashMap<String, RbacRole>>>,
    users: Arc<Mutex<HashMap<String, RbacUser>>>,
}

impl RbacManager {
    pub fn new() -> Self {
        Self {
            roles: Arc::new(Mutex::new(HashMap::new())),
            users: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a role
    pub fn register_role(&self, role: RbacRole) -> anyhow::Result<()> {
        let mut roles = self.roles.lock().unwrap();

        if roles.contains_key(&role.id) {
            return Err(anyhow::anyhow!("Role already exists: {}", role.id));
        }

        roles.insert(role.id.clone(), role);
        Ok(())
    }

    /// Get a role
    pub fn get_role(&self, role_id: &str) -> Option<RbacRole> {
        self.roles.lock().unwrap().get(role_id).cloned()
    }

    /// Update a role
    pub fn update_role(&self, role: RbacRole) -> anyhow::Result<()> {
        let mut roles = self.roles.lock().unwrap();

        if !roles.contains_key(&role.id) {
            return Err(anyhow::anyhow!("Role not found: {}", role.id));
        }

        roles.insert(role.id.clone(), role);
        Ok(())
    }

    /// Delete a role
    pub fn delete_role(&self, role_id: &str) -> anyhow::Result<()> {
        let mut roles = self.roles.lock().unwrap();

        if roles.remove(role_id).is_none() {
            return Err(anyhow::anyhow!("Role not found: {}", role_id));
        }

        Ok(())
    }

    /// List all roles
    pub fn list_roles(&self) -> Vec<RbacRole> {
        self.roles.lock().unwrap().values().cloned().collect()
    }

    /// Register a user
    pub fn register_user(&self, user: RbacUser) -> anyhow::Result<()> {
        let mut users = self.users.lock().unwrap();

        if users.contains_key(&user.id) {
            return Err(anyhow::anyhow!("User already exists: {}", user.id));
        }

        users.insert(user.id.clone(), user);
        Ok(())
    }

    /// Get a user
    pub fn get_user(&self, user_id: &str) -> Option<RbacUser> {
        self.users.lock().unwrap().get(user_id).cloned()
    }

    /// Assign role to user
    pub fn assign_role(&self, user_id: &str, role_id: &str) -> anyhow::Result<()> {
        let mut users = self.users.lock().unwrap();
        let roles = self.roles.lock().unwrap();

        // Verify role exists
        if !roles.contains_key(role_id) {
            return Err(anyhow::anyhow!("Role not found: {}", role_id));
        }

        let user = users
            .get_mut(user_id)
            .ok_or_else(|| anyhow::anyhow!("User not found: {}", user_id))?;

        user.assign_role(role_id.to_string());

        tracing::info!("Assigned role {} to user {}", role_id, user_id);

        Ok(())
    }

    /// Revoke role from user
    pub fn revoke_role(&self, user_id: &str, role_id: &str) -> anyhow::Result<()> {
        let mut users = self.users.lock().unwrap();

        let user = users
            .get_mut(user_id)
            .ok_or_else(|| anyhow::anyhow!("User not found: {}", user_id))?;

        if !user.revoke_role(role_id) {
            return Err(anyhow::anyhow!(
                "User {} does not have role {}",
                user_id,
                role_id
            ));
        }

        tracing::info!("Revoked role {} from user {}", role_id, user_id);

        Ok(())
    }

    /// Check if user has permission
    pub fn check_permission(&self, user_id: &str, permission: &Permission) -> bool {
        let users = self.users.lock().unwrap();
        let roles = self.roles.lock().unwrap();

        let user = match users.get(user_id) {
            Some(u) => u,
            None => return false,
        };

        // Check all user's roles for the permission
        for role_id in &user.roles {
            if let Some(role) = roles.get(role_id) {
                if role.has_permission(permission) {
                    return true;
                }
            }
        }

        false
    }

    /// Get all permissions for a user
    pub fn get_user_permissions(&self, user_id: &str) -> HashSet<Permission> {
        let users = self.users.lock().unwrap();
        let roles = self.roles.lock().unwrap();

        let user = match users.get(user_id) {
            Some(u) => u,
            None => return HashSet::new(),
        };

        let mut permissions = HashSet::new();

        for role_id in &user.roles {
            if let Some(role) = roles.get(role_id) {
                permissions.extend(role.permissions.clone());
            }
        }

        permissions
    }
}

impl Default for RbacManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Predefined system roles
pub fn create_system_roles() -> Vec<RbacRole> {
    vec![
        // Admin role - full access
        {
            let mut role = RbacRole::new(
                "admin".to_string(),
                "Administrator".to_string(),
                "Full system access".to_string(),
            );
            role.add_permission(Permission::new(Resource::All, Action::All));
            role
        },
        // Workflow manager
        {
            let mut role = RbacRole::new(
                "workflow_manager".to_string(),
                "Workflow Manager".to_string(),
                "Manage workflows and routines".to_string(),
            );
            role.add_permission(Permission::new(Resource::Workflow, Action::All));
            role.add_permission(Permission::new(Resource::Routine, Action::All));
            role.add_permission(Permission::new(Resource::Template, Action::Read));
            role
        },
        // Secret manager
        {
            let mut role = RbacRole::new(
                "secret_manager".to_string(),
                "Secret Manager".to_string(),
                "Manage secrets and credentials".to_string(),
            );
            role.add_permission(Permission::new(Resource::Secret, Action::All));
            role
        },
        // Auditor - read-only access to audit logs
        {
            let mut role = RbacRole::new(
                "auditor".to_string(),
                "Auditor".to_string(),
                "Read-only access to audit logs".to_string(),
            );
            role.add_permission(Permission::new(Resource::AuditLog, Action::Read));
            role.add_permission(Permission::new(Resource::AuditLog, Action::Audit));
            role
        },
        // Viewer - read-only access
        {
            let mut role = RbacRole::new(
                "viewer".to_string(),
                "Viewer".to_string(),
                "Read-only access to all resources".to_string(),
            );
            role.add_permission(Permission::new(Resource::Workflow, Action::Read));
            role.add_permission(Permission::new(Resource::Routine, Action::Read));
            role.add_permission(Permission::new(Resource::Template, Action::Read));
            role.add_permission(Permission::new(Resource::Organization, Action::Read));
            role
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_matching() {
        let perm1 = Permission::new(Resource::Workflow, Action::Read);
        let perm2 = Permission::new(Resource::Workflow, Action::Read);
        let perm3 = Permission::new(Resource::Workflow, Action::All);
        let perm4 = Permission::new(Resource::All, Action::Read);

        assert!(perm1.matches(&perm2));
        assert!(perm3.matches(&perm1)); // All action matches specific action
        assert!(perm4.matches(&perm1)); // All resource matches specific resource
    }

    #[test]
    fn test_role_permissions() {
        let mut role = RbacRole::new(
            "test".to_string(),
            "Test Role".to_string(),
            "Test".to_string(),
        );

        let perm = Permission::new(Resource::Workflow, Action::Read);
        role.add_permission(perm.clone());

        assert!(role.has_permission(&perm));
        assert!(!role.has_permission(&Permission::new(Resource::Secret, Action::Read)));
    }

    #[test]
    fn test_user_role_assignment() {
        let manager = RbacManager::new();

        let role = RbacRole::new(
            "admin".to_string(),
            "Admin".to_string(),
            "Administrator".to_string(),
        );

        manager.register_role(role).unwrap();

        let user = RbacUser::new(
            "user1".to_string(),
            "testuser".to_string(),
            "test@example.com".to_string(),
        );

        manager.register_user(user).unwrap();
        manager.assign_role("user1", "admin").unwrap();

        let retrieved_user = manager.get_user("user1").unwrap();
        assert!(retrieved_user.roles.contains("admin"));
    }

    #[test]
    fn test_permission_check() {
        let manager = RbacManager::new();

        let mut role = RbacRole::new(
            "workflow_admin".to_string(),
            "Workflow Admin".to_string(),
            "Manage workflows".to_string(),
        );
        role.add_permission(Permission::new(Resource::Workflow, Action::All));

        manager.register_role(role).unwrap();

        let user = RbacUser::new(
            "user1".to_string(),
            "testuser".to_string(),
            "test@example.com".to_string(),
        );

        manager.register_user(user).unwrap();
        manager.assign_role("user1", "workflow_admin").unwrap();

        assert!(manager.check_permission(
            "user1",
            &Permission::new(Resource::Workflow, Action::Read)
        ));
        assert!(manager.check_permission(
            "user1",
            &Permission::new(Resource::Workflow, Action::Create)
        ));
        assert!(!manager.check_permission(
            "user1",
            &Permission::new(Resource::Secret, Action::Read)
        ));
    }

    #[test]
    fn test_revoke_role() {
        let manager = RbacManager::new();

        let role = RbacRole::new(
            "admin".to_string(),
            "Admin".to_string(),
            "Administrator".to_string(),
        );

        manager.register_role(role).unwrap();

        let user = RbacUser::new(
            "user1".to_string(),
            "testuser".to_string(),
            "test@example.com".to_string(),
        );

        manager.register_user(user).unwrap();
        manager.assign_role("user1", "admin").unwrap();

        manager.revoke_role("user1", "admin").unwrap();

        let retrieved_user = manager.get_user("user1").unwrap();
        assert!(!retrieved_user.roles.contains("admin"));
    }

    #[test]
    fn test_get_user_permissions() {
        let manager = RbacManager::new();

        let mut role1 = RbacRole::new(
            "role1".to_string(),
            "Role 1".to_string(),
            "First role".to_string(),
        );
        role1.add_permission(Permission::new(Resource::Workflow, Action::Read));

        let mut role2 = RbacRole::new(
            "role2".to_string(),
            "Role 2".to_string(),
            "Second role".to_string(),
        );
        role2.add_permission(Permission::new(Resource::Secret, Action::Read));

        manager.register_role(role1).unwrap();
        manager.register_role(role2).unwrap();

        let user = RbacUser::new(
            "user1".to_string(),
            "testuser".to_string(),
            "test@example.com".to_string(),
        );

        manager.register_user(user).unwrap();
        manager.assign_role("user1", "role1").unwrap();
        manager.assign_role("user1", "role2").unwrap();

        let permissions = manager.get_user_permissions("user1");
        assert_eq!(permissions.len(), 2);
    }

    #[test]
    fn test_system_roles() {
        let roles = create_system_roles();
        assert!(roles.len() >= 5);

        let admin_role = roles.iter().find(|r| r.id == "admin").unwrap();
        assert!(admin_role.has_permission(&Permission::new(Resource::All, Action::All)));

        let viewer_role = roles.iter().find(|r| r.id == "viewer").unwrap();
        assert!(viewer_role.has_permission(&Permission::new(Resource::Workflow, Action::Read)));
        assert!(!viewer_role.has_permission(&Permission::new(Resource::Workflow, Action::Delete)));
    }
}
