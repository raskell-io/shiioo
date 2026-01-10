//! Example: Multi-tenant operations.
//!
//! This example demonstrates how to work with multiple tenants,
//! including registration, quota management, and tenant isolation.
//!
//! Run with: cargo run --example multi_tenant

use shiioo_sdk::{
    api::tenants::{RegisterTenantRequest, UpdateTenantRequest},
    TenantQuota, TenantSettings, TenantStatus,
    ShiiooClient, ShiiooResult,
};
use std::collections::HashMap;
use std::time::Duration;

#[tokio::main]
async fn main() -> ShiiooResult<()> {
    tracing_subscriber::fmt::init();

    // Create admin client (no tenant specified)
    let admin_client = ShiiooClient::builder()
        .base_url("http://localhost:8080")
        .api_key("admin-api-key")
        .build()?;

    // Register a new tenant
    println!("Registering new tenant...");
    let tenant = admin_client
        .tenants()
        .register(RegisterTenantRequest {
            name: "Acme Corp".to_string(),
            description: "Acme Corporation tenant".to_string(),
            quota: Some(TenantQuota {
                max_concurrent_workflows: Some(10),
                max_workflows_per_day: Some(100),
                max_routines: Some(20),
                max_storage_bytes: Some(1024 * 1024 * 1024), // 1GB
                max_api_requests_per_minute: Some(1000),
            }),
            settings: Some(TenantSettings {
                data_retention_days: 90,
                enable_audit_logging: true,
                metadata: HashMap::new(),
            }),
        })
        .await?;

    println!("Created tenant: {} ({})", tenant.name, tenant.id.0);
    println!("  Status: {:?}", tenant.status);
    println!(
        "  Quota: {:?} concurrent workflows, {:?} per day, {:?} bytes storage",
        tenant.quota.max_concurrent_workflows,
        tenant.quota.max_workflows_per_day,
        tenant.quota.max_storage_bytes
    );

    // List all tenants
    println!("\nListing all tenants...");
    let tenants = admin_client.tenants().list().await?;
    println!("Found {} tenants", tenants.len());

    for t in &tenants {
        let status_icon = match t.status {
            TenantStatus::Active => "[active]",
            TenantStatus::Suspended => "[suspended]",
            TenantStatus::Disabled => "[disabled]",
        };
        println!("  {} {} - {}", status_icon, t.name, t.id.0);
    }

    // Get storage statistics for the tenant
    println!("\nGetting storage stats for '{}'...", tenant.name);
    let stats = admin_client.tenants().storage_stats(&tenant.id).await?;
    println!("  Used storage: {} bytes", stats.total_bytes);
    println!("  File count: {}", stats.file_count);

    // Create a tenant-scoped client
    println!("\nCreating tenant-scoped client...");
    let tenant_client = ShiiooClient::builder()
        .base_url("http://localhost:8080")
        .api_key("tenant-api-key")
        .tenant_id(&tenant.id.0)
        .build()?;

    // Operations are now scoped to this tenant
    let runs = tenant_client.runs().list().await?;
    println!("Tenant has {} runs", runs.len());

    // Update tenant quota
    println!("\nUpdating tenant quota...");
    let updated = admin_client
        .tenants()
        .update(
            &tenant.id,
            UpdateTenantRequest {
                name: None,
                description: None,
                quota: Some(TenantQuota {
                    max_concurrent_workflows: Some(20), // Increased
                    max_workflows_per_day: Some(200),   // Increased
                    max_routines: Some(50),
                    max_storage_bytes: Some(2 * 1024 * 1024 * 1024), // 2GB
                    max_api_requests_per_minute: Some(2000),
                }),
                settings: None,
            },
        )
        .await?;
    println!(
        "Updated quota: {:?} concurrent workflows",
        updated.quota.max_concurrent_workflows
    );

    // Suspend and reactivate tenant
    println!("\nSuspending tenant...");
    let suspended = admin_client.tenants().suspend(&tenant.id).await?;
    println!("Tenant status: {:?}", suspended.status);

    tokio::time::sleep(Duration::from_secs(1)).await;

    println!("Reactivating tenant...");
    let activated = admin_client.tenants().activate(&tenant.id).await?;
    println!("Tenant status: {:?}", activated.status);

    println!("\nMulti-tenant example completed!");
    Ok(())
}
