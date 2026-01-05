use crate::types::{
    Approval, ApprovalBoard, ApprovalBoardId, ApprovalId, CapacitySource, CapacitySourceId,
    CapacityUsage, ConfigChange, ConfigChangeId, OrgId, Organization, PolicyId, PolicySpec,
    ProcessTemplate, RoleId, RoleSpec, Routine, RoutineExecution, RoutineId, Run, RunId,
    RunStatus, TemplateId,
};
use anyhow::{Context, Result};
use redb::{Database, ReadableTable, TableDefinition};
use std::path::PathBuf;
use std::sync::Arc;

const RUNS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("runs");
const ROLES_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("roles");
const POLICIES_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("policies");
const ORGS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("organizations");
const TEMPLATES_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("templates");
const CAPACITY_SOURCES_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("capacity_sources");
const CAPACITY_USAGE_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("capacity_usage");
const ROUTINES_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("routines");
const ROUTINE_EXECUTIONS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("routine_executions");
const APPROVAL_BOARDS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("approval_boards");
const APPROVALS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("approvals");
const CONFIG_CHANGES_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("config_changes");

/// Index store for fast queries using redb
#[derive(Clone)]
pub struct RedbIndexStore {
    db: Arc<Database>,
}

impl RedbIndexStore {
    pub fn new(path: PathBuf) -> Result<Self> {
        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("Failed to create index directory")?;
        }

        let db = Database::create(&path).context("Failed to create redb database")?;

        // Initialize tables
        let write_txn = db.begin_write().context("Failed to begin write transaction")?;
        {
            let _runs_table = write_txn
                .open_table(RUNS_TABLE)
                .context("Failed to open runs table")?;
            let _roles_table = write_txn
                .open_table(ROLES_TABLE)
                .context("Failed to open roles table")?;
            let _policies_table = write_txn
                .open_table(POLICIES_TABLE)
                .context("Failed to open policies table")?;
            let _orgs_table = write_txn
                .open_table(ORGS_TABLE)
                .context("Failed to open orgs table")?;
            let _templates_table = write_txn
                .open_table(TEMPLATES_TABLE)
                .context("Failed to open templates table")?;
            let _capacity_sources_table = write_txn
                .open_table(CAPACITY_SOURCES_TABLE)
                .context("Failed to open capacity sources table")?;
            let _capacity_usage_table = write_txn
                .open_table(CAPACITY_USAGE_TABLE)
                .context("Failed to open capacity usage table")?;
            let _routines_table = write_txn
                .open_table(ROUTINES_TABLE)
                .context("Failed to open routines table")?;
            let _routine_executions_table = write_txn
                .open_table(ROUTINE_EXECUTIONS_TABLE)
                .context("Failed to open routine executions table")?;
            let _approval_boards_table = write_txn
                .open_table(APPROVAL_BOARDS_TABLE)
                .context("Failed to open approval boards table")?;
            let _approvals_table = write_txn
                .open_table(APPROVALS_TABLE)
                .context("Failed to open approvals table")?;
            let _config_changes_table = write_txn
                .open_table(CONFIG_CHANGES_TABLE)
                .context("Failed to open config changes table")?;
        }
        write_txn.commit().context("Failed to commit transaction")?;

        Ok(Self { db: Arc::new(db) })
    }

    /// Index a run for fast queries
    pub fn index_run(&self, run: &Run) -> Result<()> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        {
            let mut table = write_txn
                .open_table(RUNS_TABLE)
                .context("Failed to open table")?;

            let key = run.id.to_string();
            let value = serde_json::to_vec(run).context("Failed to serialize run")?;

            table
                .insert(key.as_str(), value.as_slice())
                .context("Failed to insert run")?;
        }
        write_txn.commit().context("Failed to commit")?;
        Ok(())
    }

    /// Get a run by ID
    pub fn get_run(&self, run_id: &RunId) -> Result<Option<Run>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(RUNS_TABLE).context("Failed to open table")?;

        let key = run_id.to_string();
        let value = table.get(key.as_str()).context("Failed to get run")?;

        match value {
            Some(guard) => {
                let bytes = guard.value();
                let run: Run = serde_json::from_slice(bytes).context("Failed to deserialize run")?;
                Ok(Some(run))
            }
            None => Ok(None),
        }
    }

    /// List all runs (for MVP - in production this would need pagination)
    pub fn list_runs(&self) -> Result<Vec<Run>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(RUNS_TABLE).context("Failed to open table")?;

        let mut runs = Vec::new();
        for item in table.iter().context("Failed to iterate runs")? {
            let (_key, value) = item.context("Failed to read item")?;
            let run: Run = serde_json::from_slice(value.value())
                .context("Failed to deserialize run")?;
            runs.push(run);
        }

        // Sort by started_at descending (most recent first)
        runs.sort_by(|a, b| b.started_at.cmp(&a.started_at));

        Ok(runs)
    }

    /// Update run status
    pub fn update_run_status(&self, run_id: &RunId, status: RunStatus) -> Result<()> {
        let mut run = self
            .get_run(run_id)?
            .context("Run not found")?;

        run.status = status;
        if matches!(
            status,
            RunStatus::Completed | RunStatus::Failed | RunStatus::Cancelled
        ) {
            run.completed_at = Some(chrono::Utc::now());
        }

        self.index_run(&run)
    }

    /// Store a role
    pub fn store_role(&self, role: &RoleSpec) -> Result<()> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        {
            let mut table = write_txn
                .open_table(ROLES_TABLE)
                .context("Failed to open table")?;

            let key = &role.id.0;
            let value = serde_json::to_vec(role).context("Failed to serialize role")?;

            table
                .insert(key.as_str(), value.as_slice())
                .context("Failed to insert role")?;
        }
        write_txn.commit().context("Failed to commit")?;
        Ok(())
    }

    /// Get a role by ID
    pub fn get_role(&self, role_id: &RoleId) -> Result<Option<RoleSpec>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(ROLES_TABLE).context("Failed to open table")?;

        let value = table.get(role_id.0.as_str()).context("Failed to get role")?;

        match value {
            Some(guard) => {
                let bytes = guard.value();
                let role: RoleSpec = serde_json::from_slice(bytes).context("Failed to deserialize role")?;
                Ok(Some(role))
            }
            None => Ok(None),
        }
    }

    /// List all roles
    pub fn list_roles(&self) -> Result<Vec<RoleSpec>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(ROLES_TABLE).context("Failed to open table")?;

        let mut roles = Vec::new();
        for item in table.iter().context("Failed to iterate roles")? {
            let (_key, value) = item.context("Failed to read item")?;
            let role: RoleSpec = serde_json::from_slice(value.value())
                .context("Failed to deserialize role")?;
            roles.push(role);
        }

        Ok(roles)
    }

    /// Delete a role
    pub fn delete_role(&self, role_id: &RoleId) -> Result<()> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        {
            let mut table = write_txn
                .open_table(ROLES_TABLE)
                .context("Failed to open table")?;

            table
                .remove(role_id.0.as_str())
                .context("Failed to delete role")?;
        }
        write_txn.commit().context("Failed to commit")?;
        Ok(())
    }

    /// Store a policy
    pub fn store_policy(&self, policy: &PolicySpec) -> Result<()> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        {
            let mut table = write_txn
                .open_table(POLICIES_TABLE)
                .context("Failed to open table")?;

            let key = &policy.id.0;
            let value = serde_json::to_vec(policy).context("Failed to serialize policy")?;

            table
                .insert(key.as_str(), value.as_slice())
                .context("Failed to insert policy")?;
        }
        write_txn.commit().context("Failed to commit")?;
        Ok(())
    }

    /// Get a policy by ID
    pub fn get_policy(&self, policy_id: &PolicyId) -> Result<Option<PolicySpec>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(POLICIES_TABLE).context("Failed to open table")?;

        let value = table.get(policy_id.0.as_str()).context("Failed to get policy")?;

        match value {
            Some(guard) => {
                let bytes = guard.value();
                let policy: PolicySpec = serde_json::from_slice(bytes).context("Failed to deserialize policy")?;
                Ok(Some(policy))
            }
            None => Ok(None),
        }
    }

    /// List all policies
    pub fn list_policies(&self) -> Result<Vec<PolicySpec>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(POLICIES_TABLE).context("Failed to open table")?;

        let mut policies = Vec::new();
        for item in table.iter().context("Failed to iterate policies")? {
            let (_key, value) = item.context("Failed to read item")?;
            let policy: PolicySpec = serde_json::from_slice(value.value())
                .context("Failed to deserialize policy")?;
            policies.push(policy);
        }

        Ok(policies)
    }

    /// Delete a policy
    pub fn delete_policy(&self, policy_id: &PolicyId) -> Result<()> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        {
            let mut table = write_txn
                .open_table(POLICIES_TABLE)
                .context("Failed to open table")?;

            table
                .remove(policy_id.0.as_str())
                .context("Failed to delete policy")?;
        }
        write_txn.commit().context("Failed to commit")?;
        Ok(())
    }

    /// Store an organization
    pub fn store_organization(&self, org: &Organization) -> Result<()> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        {
            let mut table = write_txn
                .open_table(ORGS_TABLE)
                .context("Failed to open table")?;

            let key = &org.id.0;
            let value = serde_json::to_vec(org).context("Failed to serialize organization")?;

            table
                .insert(key.as_str(), value.as_slice())
                .context("Failed to insert organization")?;
        }
        write_txn.commit().context("Failed to commit")?;
        Ok(())
    }

    /// Get an organization by ID
    pub fn get_organization(&self, org_id: &OrgId) -> Result<Option<Organization>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(ORGS_TABLE).context("Failed to open table")?;

        let value = table.get(org_id.0.as_str()).context("Failed to get organization")?;

        match value {
            Some(guard) => {
                let bytes = guard.value();
                let org: Organization = serde_json::from_slice(bytes).context("Failed to deserialize organization")?;
                Ok(Some(org))
            }
            None => Ok(None),
        }
    }

    /// List all organizations
    pub fn list_organizations(&self) -> Result<Vec<Organization>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(ORGS_TABLE).context("Failed to open table")?;

        let mut orgs = Vec::new();
        for item in table.iter().context("Failed to iterate organizations")? {
            let (_key, value) = item.context("Failed to read item")?;
            let org: Organization = serde_json::from_slice(value.value())
                .context("Failed to deserialize organization")?;
            orgs.push(org);
        }

        Ok(orgs)
    }

    /// Delete an organization
    pub fn delete_organization(&self, org_id: &OrgId) -> Result<()> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        {
            let mut table = write_txn
                .open_table(ORGS_TABLE)
                .context("Failed to open table")?;

            table
                .remove(org_id.0.as_str())
                .context("Failed to delete organization")?;
        }
        write_txn.commit().context("Failed to commit")?;
        Ok(())
    }

    /// Store a process template
    pub fn store_template(&self, template: &ProcessTemplate) -> Result<()> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        {
            let mut table = write_txn
                .open_table(TEMPLATES_TABLE)
                .context("Failed to open table")?;

            let key = &template.id.0;
            let value = serde_json::to_vec(template).context("Failed to serialize template")?;

            table
                .insert(key.as_str(), value.as_slice())
                .context("Failed to insert template")?;
        }
        write_txn.commit().context("Failed to commit")?;
        Ok(())
    }

    /// Get a template by ID
    pub fn get_template(&self, template_id: &TemplateId) -> Result<Option<ProcessTemplate>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(TEMPLATES_TABLE).context("Failed to open table")?;

        let value = table.get(template_id.0.as_str()).context("Failed to get template")?;

        match value {
            Some(guard) => {
                let bytes = guard.value();
                let template: ProcessTemplate = serde_json::from_slice(bytes).context("Failed to deserialize template")?;
                Ok(Some(template))
            }
            None => Ok(None),
        }
    }

    /// List all templates
    pub fn list_templates(&self) -> Result<Vec<ProcessTemplate>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(TEMPLATES_TABLE).context("Failed to open table")?;

        let mut templates = Vec::new();
        for item in table.iter().context("Failed to iterate templates")? {
            let (_key, value) = item.context("Failed to read item")?;
            let template: ProcessTemplate = serde_json::from_slice(value.value())
                .context("Failed to deserialize template")?;
            templates.push(template);
        }

        Ok(templates)
    }

    /// Delete a template
    pub fn delete_template(&self, template_id: &TemplateId) -> Result<()> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        {
            let mut table = write_txn
                .open_table(TEMPLATES_TABLE)
                .context("Failed to open table")?;

            table
                .remove(template_id.0.as_str())
                .context("Failed to delete template")?;
        }
        write_txn.commit().context("Failed to commit")?;
        Ok(())
    }

    /// Store a capacity source
    pub fn store_capacity_source(&self, source: &CapacitySource) -> Result<()> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        {
            let mut table = write_txn
                .open_table(CAPACITY_SOURCES_TABLE)
                .context("Failed to open table")?;

            let key = &source.id.0;
            let value = serde_json::to_vec(source).context("Failed to serialize capacity source")?;

            table
                .insert(key.as_str(), value.as_slice())
                .context("Failed to insert capacity source")?;
        }
        write_txn.commit().context("Failed to commit")?;
        Ok(())
    }

    /// Get a capacity source by ID
    pub fn get_capacity_source(&self, source_id: &CapacitySourceId) -> Result<Option<CapacitySource>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(CAPACITY_SOURCES_TABLE).context("Failed to open table")?;

        let value = table.get(source_id.0.as_str()).context("Failed to get capacity source")?;

        match value {
            Some(guard) => {
                let bytes = guard.value();
                let source: CapacitySource = serde_json::from_slice(bytes)
                    .context("Failed to deserialize capacity source")?;
                Ok(Some(source))
            }
            None => Ok(None),
        }
    }

    /// List all capacity sources
    pub fn list_capacity_sources(&self) -> Result<Vec<CapacitySource>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(CAPACITY_SOURCES_TABLE).context("Failed to open table")?;

        let mut sources = Vec::new();
        for item in table.iter().context("Failed to iterate capacity sources")? {
            let (_key, value) = item.context("Failed to read item")?;
            let source: CapacitySource = serde_json::from_slice(value.value())
                .context("Failed to deserialize capacity source")?;
            sources.push(source);
        }

        Ok(sources)
    }

    /// Delete a capacity source
    pub fn delete_capacity_source(&self, source_id: &CapacitySourceId) -> Result<()> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        {
            let mut table = write_txn
                .open_table(CAPACITY_SOURCES_TABLE)
                .context("Failed to open table")?;

            table
                .remove(source_id.0.as_str())
                .context("Failed to delete capacity source")?;
        }
        write_txn.commit().context("Failed to commit")?;
        Ok(())
    }

    /// Store capacity usage record
    pub fn store_capacity_usage(&self, usage: &CapacityUsage) -> Result<()> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        {
            let mut table = write_txn
                .open_table(CAPACITY_USAGE_TABLE)
                .context("Failed to open table")?;

            let key = &usage.id;
            let value = serde_json::to_vec(usage).context("Failed to serialize capacity usage")?;

            table
                .insert(key.as_str(), value.as_slice())
                .context("Failed to insert capacity usage")?;
        }
        write_txn.commit().context("Failed to commit")?;
        Ok(())
    }

    /// List all capacity usage records
    pub fn list_capacity_usage(&self) -> Result<Vec<CapacityUsage>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(CAPACITY_USAGE_TABLE).context("Failed to open table")?;

        let mut usage_records = Vec::new();
        for item in table.iter().context("Failed to iterate capacity usage")? {
            let (_key, value) = item.context("Failed to read item")?;
            let usage: CapacityUsage = serde_json::from_slice(value.value())
                .context("Failed to deserialize capacity usage")?;
            usage_records.push(usage);
        }

        // Sort by timestamp descending (most recent first)
        usage_records.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        Ok(usage_records)
    }

    /// Store a routine
    pub fn store_routine(&self, routine: &Routine) -> Result<()> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        {
            let mut table = write_txn
                .open_table(ROUTINES_TABLE)
                .context("Failed to open table")?;

            let key = &routine.id.0;
            let value = serde_json::to_vec(routine).context("Failed to serialize routine")?;

            table
                .insert(key.as_str(), value.as_slice())
                .context("Failed to insert routine")?;
        }
        write_txn.commit().context("Failed to commit")?;
        Ok(())
    }

    /// Get a routine by ID
    pub fn get_routine(&self, routine_id: &RoutineId) -> Result<Option<Routine>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(ROUTINES_TABLE).context("Failed to open table")?;

        let value = table.get(routine_id.0.as_str()).context("Failed to get routine")?;

        match value {
            Some(guard) => {
                let bytes = guard.value();
                let routine: Routine = serde_json::from_slice(bytes).context("Failed to deserialize routine")?;
                Ok(Some(routine))
            }
            None => Ok(None),
        }
    }

    /// List all routines
    pub fn list_routines(&self) -> Result<Vec<Routine>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(ROUTINES_TABLE).context("Failed to open table")?;

        let mut routines = Vec::new();
        for item in table.iter().context("Failed to iterate routines")? {
            let (_key, value) = item.context("Failed to read item")?;
            let routine: Routine = serde_json::from_slice(value.value())
                .context("Failed to deserialize routine")?;
            routines.push(routine);
        }

        Ok(routines)
    }

    /// Delete a routine
    pub fn delete_routine(&self, routine_id: &RoutineId) -> Result<()> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        {
            let mut table = write_txn
                .open_table(ROUTINES_TABLE)
                .context("Failed to open table")?;

            table
                .remove(routine_id.0.as_str())
                .context("Failed to delete routine")?;
        }
        write_txn.commit().context("Failed to commit")?;
        Ok(())
    }

    /// Store routine execution
    pub fn store_routine_execution(&self, execution: &RoutineExecution) -> Result<()> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        {
            let mut table = write_txn
                .open_table(ROUTINE_EXECUTIONS_TABLE)
                .context("Failed to open table")?;

            let key = &execution.id;
            let value = serde_json::to_vec(execution).context("Failed to serialize execution")?;

            table
                .insert(key.as_str(), value.as_slice())
                .context("Failed to insert execution")?;
        }
        write_txn.commit().context("Failed to commit")?;
        Ok(())
    }

    /// List routine executions
    pub fn list_routine_executions(&self) -> Result<Vec<RoutineExecution>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(ROUTINE_EXECUTIONS_TABLE).context("Failed to open table")?;

        let mut executions = Vec::new();
        for item in table.iter().context("Failed to iterate executions")? {
            let (_key, value) = item.context("Failed to read item")?;
            let execution: RoutineExecution = serde_json::from_slice(value.value())
                .context("Failed to deserialize execution")?;
            executions.push(execution);
        }

        executions.sort_by(|a, b| b.executed_at.cmp(&a.executed_at));
        Ok(executions)
    }

    /// Store approval board
    pub fn store_approval_board(&self, board: &ApprovalBoard) -> Result<()> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        {
            let mut table = write_txn
                .open_table(APPROVAL_BOARDS_TABLE)
                .context("Failed to open table")?;

            let key = &board.id.0;
            let value = serde_json::to_vec(board).context("Failed to serialize board")?;

            table
                .insert(key.as_str(), value.as_slice())
                .context("Failed to insert board")?;
        }
        write_txn.commit().context("Failed to commit")?;
        Ok(())
    }

    /// Get approval board by ID
    pub fn get_approval_board(&self, board_id: &ApprovalBoardId) -> Result<Option<ApprovalBoard>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(APPROVAL_BOARDS_TABLE).context("Failed to open table")?;

        let value = table.get(board_id.0.as_str()).context("Failed to get board")?;

        match value {
            Some(guard) => {
                let bytes = guard.value();
                let board: ApprovalBoard = serde_json::from_slice(bytes).context("Failed to deserialize board")?;
                Ok(Some(board))
            }
            None => Ok(None),
        }
    }

    /// List all approval boards
    pub fn list_approval_boards(&self) -> Result<Vec<ApprovalBoard>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(APPROVAL_BOARDS_TABLE).context("Failed to open table")?;

        let mut boards = Vec::new();
        for item in table.iter().context("Failed to iterate boards")? {
            let (_key, value) = item.context("Failed to read item")?;
            let board: ApprovalBoard = serde_json::from_slice(value.value())
                .context("Failed to deserialize board")?;
            boards.push(board);
        }

        Ok(boards)
    }

    /// Delete approval board
    pub fn delete_approval_board(&self, board_id: &ApprovalBoardId) -> Result<()> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        {
            let mut table = write_txn
                .open_table(APPROVAL_BOARDS_TABLE)
                .context("Failed to open table")?;

            table
                .remove(board_id.0.as_str())
                .context("Failed to delete board")?;
        }
        write_txn.commit().context("Failed to commit")?;
        Ok(())
    }

    /// Store approval
    pub fn store_approval(&self, approval: &Approval) -> Result<()> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        {
            let mut table = write_txn
                .open_table(APPROVALS_TABLE)
                .context("Failed to open table")?;

            let key = &approval.id.0;
            let value = serde_json::to_vec(approval).context("Failed to serialize approval")?;

            table
                .insert(key.as_str(), value.as_slice())
                .context("Failed to insert approval")?;
        }
        write_txn.commit().context("Failed to commit")?;
        Ok(())
    }

    /// Get approval by ID
    pub fn get_approval(&self, approval_id: &ApprovalId) -> Result<Option<Approval>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(APPROVALS_TABLE).context("Failed to open table")?;

        let value = table.get(approval_id.0.as_str()).context("Failed to get approval")?;

        match value {
            Some(guard) => {
                let bytes = guard.value();
                let approval: Approval = serde_json::from_slice(bytes).context("Failed to deserialize approval")?;
                Ok(Some(approval))
            }
            None => Ok(None),
        }
    }

    /// List all approvals
    pub fn list_approvals(&self) -> Result<Vec<Approval>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(APPROVALS_TABLE).context("Failed to open table")?;

        let mut approvals = Vec::new();
        for item in table.iter().context("Failed to iterate approvals")? {
            let (_key, value) = item.context("Failed to read item")?;
            let approval: Approval = serde_json::from_slice(value.value())
                .context("Failed to deserialize approval")?;
            approvals.push(approval);
        }

        approvals.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(approvals)
    }

    /// Store config change
    pub fn store_config_change(&self, change: &ConfigChange) -> Result<()> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        {
            let mut table = write_txn
                .open_table(CONFIG_CHANGES_TABLE)
                .context("Failed to open table")?;

            let key = &change.id.0;
            let value = serde_json::to_vec(change).context("Failed to serialize change")?;

            table
                .insert(key.as_str(), value.as_slice())
                .context("Failed to insert change")?;
        }
        write_txn.commit().context("Failed to commit")?;
        Ok(())
    }

    /// Get config change by ID
    pub fn get_config_change(&self, change_id: &ConfigChangeId) -> Result<Option<ConfigChange>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(CONFIG_CHANGES_TABLE).context("Failed to open table")?;

        let value = table.get(change_id.0.as_str()).context("Failed to get change")?;

        match value {
            Some(guard) => {
                let bytes = guard.value();
                let change: ConfigChange = serde_json::from_slice(bytes).context("Failed to deserialize change")?;
                Ok(Some(change))
            }
            None => Ok(None),
        }
    }

    /// List all config changes
    pub fn list_config_changes(&self) -> Result<Vec<ConfigChange>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn.open_table(CONFIG_CHANGES_TABLE).context("Failed to open table")?;

        let mut changes = Vec::new();
        for item in table.iter().context("Failed to iterate changes")? {
            let (_key, value) = item.context("Failed to read item")?;
            let change: ConfigChange = serde_json::from_slice(value.value())
                .context("Failed to deserialize change")?;
            changes.push(change);
        }

        changes.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(changes)
    }
}

/// Trait for index storage
pub trait IndexStore: Send + Sync {
    /// Index a run
    fn index_run(&self, run: &Run) -> Result<()>;

    /// Get a run by ID
    fn get_run(&self, run_id: &RunId) -> Result<Option<Run>>;

    /// List all runs
    fn list_runs(&self) -> Result<Vec<Run>>;

    /// Update run status
    fn update_run_status(&self, run_id: &RunId, status: RunStatus) -> Result<()>;
}

impl IndexStore for RedbIndexStore {
    fn index_run(&self, run: &Run) -> Result<()> {
        RedbIndexStore::index_run(self, run)
    }

    fn get_run(&self, run_id: &RunId) -> Result<Option<Run>> {
        RedbIndexStore::get_run(self, run_id)
    }

    fn list_runs(&self) -> Result<Vec<Run>> {
        RedbIndexStore::list_runs(self)
    }

    fn update_run_status(&self, run_id: &RunId, status: RunStatus) -> Result<()> {
        RedbIndexStore::update_run_status(self, run_id, status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_redb_index_store() {
        let temp_file = NamedTempFile::new().unwrap();
        let store = RedbIndexStore::new(temp_file.path().to_path_buf()).unwrap();

        let run = Run {
            id: RunId::new(),
            work_item_id: "test-job".to_string(),
            status: RunStatus::Running,
            started_at: chrono::Utc::now(),
            completed_at: None,
            steps: vec![],
        };

        store.index_run(&run).unwrap();

        let retrieved = store.get_run(&run.id).unwrap().unwrap();
        assert_eq!(retrieved.id, run.id);

        let runs = store.list_runs().unwrap();
        assert_eq!(runs.len(), 1);

        store
            .update_run_status(&run.id, RunStatus::Completed)
            .unwrap();
        let updated = store.get_run(&run.id).unwrap().unwrap();
        assert_eq!(updated.status, RunStatus::Completed);
        assert!(updated.completed_at.is_some());
    }
}
