use crate::types::{Run, RunId, RunStatus};
use anyhow::{Context, Result};
use redb::{Database, ReadableTable, TableDefinition};
use std::path::PathBuf;
use std::sync::Arc;

const RUNS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("runs");

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
            let _table = write_txn
                .open_table(RUNS_TABLE)
                .context("Failed to open runs table")?;
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
