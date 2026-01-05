use crate::events::{Event, EventLog};
use crate::types::RunId;
use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Utc};
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Write;
use std::path::PathBuf;
use tokio::sync::RwLock;

/// Event log implementation using JSONL (JSON Lines) format with optional compression
pub struct JsonlEventLog {
    base_path: PathBuf,
    // In-memory buffer for the current day's events (flushed periodically)
    buffer: RwLock<Vec<Event>>,
}

impl JsonlEventLog {
    pub fn new(base_path: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&base_path)
            .context("Failed to create event log directory")?;
        Ok(Self {
            base_path,
            buffer: RwLock::new(Vec::new()),
        })
    }

    /// Get the path to the event log file for a specific run
    /// Format: events/YYYY/MM/DD/<run_id>.jsonl.gz
    fn event_log_path(&self, run_id: &RunId, date: &DateTime<Utc>) -> PathBuf {
        self.base_path
            .join("events")
            .join(format!("{:04}", date.year()))
            .join(format!("{:02}", date.month()))
            .join(format!("{:02}", date.day()))
            .join(format!("{}.jsonl.gz", run_id))
    }

    /// Flush buffered events to disk
    async fn flush(&self, run_id: &RunId) -> Result<()> {
        let mut buffer = self.buffer.write().await;
        if buffer.is_empty() {
            return Ok(());
        }

        // Group events by date
        let mut events_by_date: std::collections::HashMap<DateTime<Utc>, Vec<Event>> =
            std::collections::HashMap::new();

        for event in buffer.drain(..) {
            let date = event.timestamp.date_naive().and_hms_opt(0, 0, 0).unwrap();
            let date_utc = DateTime::<Utc>::from_naive_utc_and_offset(date, Utc);
            events_by_date.entry(date_utc).or_default().push(event);
        }

        // Write each date's events to its own file
        for (date, events) in events_by_date {
            let path = self.event_log_path(run_id, &date);

            // Create parent directory
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .context("Failed to create event log directory")?;
            }

            // Read existing events if file exists
            let mut all_events = if path.exists() {
                self.read_jsonl_gz(&path).await?
            } else {
                Vec::new()
            };

            all_events.extend(events);

            // Write all events to compressed JSONL
            self.write_jsonl_gz(&path, &all_events).await?;
        }

        Ok(())
    }

    /// Read JSONL.GZ file
    async fn read_jsonl_gz(&self, path: &PathBuf) -> Result<Vec<Event>> {
        use flate2::read::GzDecoder;
        use std::io::BufRead;

        let file = std::fs::File::open(path).context("Failed to open event log")?;
        let decoder = GzDecoder::new(file);
        let reader = std::io::BufReader::new(decoder);

        let mut events = Vec::new();
        for line in reader.lines() {
            let line = line.context("Failed to read line from event log")?;
            let event: Event = serde_json::from_str(&line).context("Failed to parse event")?;
            events.push(event);
        }

        Ok(events)
    }

    /// Write JSONL.GZ file
    async fn write_jsonl_gz(&self, path: &PathBuf, events: &[Event]) -> Result<()> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());

        for event in events {
            let json = serde_json::to_string(event).context("Failed to serialize event")?;
            encoder
                .write_all(json.as_bytes())
                .context("Failed to write event")?;
            encoder.write_all(b"\n").context("Failed to write newline")?;
        }

        let compressed = encoder.finish().context("Failed to finish compression")?;

        tokio::fs::write(path, compressed)
            .await
            .context("Failed to write event log file")?;

        Ok(())
    }

    /// Get all event log files for a run
    async fn get_log_files(&self, run_id: &RunId) -> Result<Vec<PathBuf>> {
        let events_dir = self.base_path.join("events");
        if !events_dir.exists() {
            return Ok(Vec::new());
        }

        let mut files = Vec::new();
        let filename = format!("{}.jsonl.gz", run_id);

        // Walk through year/month/day directories
        for year_entry in std::fs::read_dir(&events_dir)
            .context("Failed to read events directory")?
        {
            let year_entry = year_entry.context("Failed to read year entry")?;
            if !year_entry.path().is_dir() {
                continue;
            }

            for month_entry in std::fs::read_dir(year_entry.path())
                .context("Failed to read month directory")?
            {
                let month_entry = month_entry.context("Failed to read month entry")?;
                if !month_entry.path().is_dir() {
                    continue;
                }

                for day_entry in std::fs::read_dir(month_entry.path())
                    .context("Failed to read day directory")?
                {
                    let day_entry = day_entry.context("Failed to read day entry")?;
                    if !day_entry.path().is_dir() {
                        continue;
                    }

                    let log_file = day_entry.path().join(&filename);
                    if log_file.exists() {
                        files.push(log_file);
                    }
                }
            }
        }

        Ok(files)
    }
}

#[async_trait::async_trait]
impl EventLog for JsonlEventLog {
    async fn append(&self, event: Event) -> Result<()> {
        let run_id = event.run_id;
        let mut buffer = self.buffer.write().await;
        buffer.push(event);

        // Flush buffer if it gets large (e.g., > 100 events)
        if buffer.len() > 100 {
            drop(buffer);
            self.flush(&run_id).await?;
        }

        Ok(())
    }

    async fn get_run_events(&self, run_id: RunId) -> Result<Vec<Event>> {
        // Flush any buffered events first
        self.flush(&run_id).await?;

        let log_files = self.get_log_files(&run_id).await?;
        let mut all_events = Vec::new();

        for file in log_files {
            let events = self.read_jsonl_gz(&file).await?;
            all_events.extend(events);
        }

        // Sort by timestamp
        all_events.sort_by_key(|e| e.timestamp);

        Ok(all_events)
    }

    async fn get_run_events_range(
        &self,
        run_id: RunId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Event>> {
        let all_events = self.get_run_events(run_id).await?;
        Ok(all_events
            .into_iter()
            .filter(|e| e.timestamp >= start && e.timestamp <= end)
            .collect())
    }
}

/// Trait for event log storage
#[async_trait::async_trait]
pub trait EventLogStore: Send + Sync {
    /// Get the event log
    fn event_log(&self) -> &dyn EventLog;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventType;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_jsonl_event_log() {
        let temp_dir = TempDir::new().unwrap();
        let log = JsonlEventLog::new(temp_dir.path().to_path_buf()).unwrap();

        let run_id = RunId::new();
        let event = Event::new(
            run_id,
            EventType::RunStarted {
                work_item_id: "test-job".to_string(),
                workflow_spec: crate::types::WorkflowSpec {
                    steps: vec![],
                    dependencies: std::collections::HashMap::new(),
                },
            },
        );

        log.append(event.clone()).await.unwrap();
        log.flush(&run_id).await.unwrap();

        let events = log.get_run_events(run_id).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, event.id);
    }
}
