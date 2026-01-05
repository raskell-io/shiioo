use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
};
use serde::{Deserialize, Serialize};
use shiioo_core::rbac::{Action, Permission, RbacManager, Resource};
use std::sync::Arc;

/// Authentication token claims
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthClaims {
    pub user_id: String,
    pub email: String,
    pub roles: Vec<String>,
    pub exp: i64,
}

/// Extract user ID from authorization header
pub fn extract_user_from_headers(headers: &HeaderMap) -> Option<String> {
    let auth_header = headers.get("Authorization")?;
    let auth_str = auth_header.to_str().ok()?;

    if auth_str.starts_with("Bearer ") {
        // In production, this would verify JWT and extract user ID
        // For now, we'll use a simple extraction
        Some("demo-user".to_string())
    } else {
        None
    }
}

/// Check if user has permission
pub fn check_permission(
    rbac_manager: &RbacManager,
    user_id: &str,
    resource: Resource,
    action: Action,
) -> bool {
    let permission = Permission::new(resource, action);
    rbac_manager.check_permission(user_id, &permission)
}

/// Permission enforcement middleware
pub async fn require_permission(
    resource: Resource,
    action: Action,
) -> impl Fn(Request, Next) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Response, StatusCode>> + Send>> {
    move |req: Request, next: Next| {
        let resource = resource.clone();
        let action = action.clone();

        Box::pin(async move {
            // Extract user from request headers
            let user_id = match extract_user_from_headers(req.headers()) {
                Some(id) => id,
                None => return Err(StatusCode::UNAUTHORIZED),
            };

            // TODO: Get RbacManager from app state and check permission
            // For now, we'll allow all requests in the middleware
            // The actual permission check will be done in handlers

            Ok(next.run(req).await)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn test_extract_user_from_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            HeaderValue::from_static("Bearer token123"),
        );

        let user_id = extract_user_from_headers(&headers);
        assert!(user_id.is_some());
    }

    #[test]
    fn test_check_permission() {
        let rbac_manager = RbacManager::new();

        // Create a role with permissions
        let mut role = shiioo_core::rbac::RbacRole::new(
            "admin".to_string(),
            "Admin".to_string(),
            "Administrator".to_string(),
        );
        role.add_permission(Permission::new(Resource::All, Action::All));

        rbac_manager.register_role(role).unwrap();

        // Create a user and assign the role
        let user = shiioo_core::rbac::RbacUser::new(
            "user1".to_string(),
            "admin".to_string(),
            "admin@example.com".to_string(),
        );

        rbac_manager.register_user(user).unwrap();
        rbac_manager.assign_role("user1", "admin").unwrap();

        // Check permission
        assert!(check_permission(
            &rbac_manager,
            "user1",
            Resource::Workflow,
            Action::Create
        ));
    }
}
