//! The machine where tools run, in-process for on-device use. The remote
//! `shiioo daemon` will implement the same contract over the wire
//! (`docs/design/tools-and-machines.md`).
//!
//! Two properties make tool execution safe across worker crashes:
//!
//! - **A journal of calls.** Every call is recorded as started before it
//!   runs and as finished (with its output) after. A redelivered call is
//!   answered from the journal instead of running again, and a new worker
//!   can ask whether a call ran instead of guessing.
//! - **Fencing at the machine.** Every invocation carries the lease epoch
//!   of the worker that sent it. The machine remembers the highest epoch it
//!   has seen per run and refuses older ones, so a stalled worker that wakes
//!   up after a takeover cannot run a tool the new worker already handled.

use crate::tools::{ToolOutput, Toolbox};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shiioo_core::{CallId, Epoch, RunId};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

#[derive(Clone, Copy, Debug)]
pub struct Invocation<'a> {
    pub run_id: RunId,
    /// Epoch of the sending worker's lease.
    pub epoch: Epoch,
    pub call_id: &'a CallId,
    pub tool: &'a str,
    pub input: &'a Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CallStatus {
    /// The machine never received this call. It did not run.
    NotReceived,
    /// The call started but did not finish: the machine stopped while it
    /// ran. Whether its effects happened is unknown.
    Started,
    Finished(ToolOutput),
}

#[derive(Debug, thiserror::Error)]
pub enum MachineError {
    #[error("fenced: invocation from epoch {presented:?}, this run is at {current:?}")]
    Fenced { presented: Epoch, current: Epoch },
    #[error("call {0:?} started earlier and its outcome is unknown")]
    OutcomeUnknown(CallId),
    #[error("unknown tool '{0}'")]
    UnknownTool(String),
    #[error("journal: {0}")]
    Journal(String),
    #[error("simulated crash before the call was journaled as finished")]
    SimulatedCrash,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Record {
    Fence {
        run: RunId,
        epoch: Epoch,
    },
    Started {
        run: RunId,
        call: CallId,
    },
    Finished {
        run: RunId,
        call: CallId,
        output: ToolOutput,
    },
    /// A pure or idempotent call that started but never finished is being
    /// run again.
    Reset {
        run: RunId,
        call: CallId,
    },
}

enum Entry {
    Started,
    Finished(ToolOutput),
}

#[derive(Default)]
struct Journal {
    file: Option<File>,
    calls: HashMap<(RunId, CallId), Entry>,
    fences: HashMap<RunId, Epoch>,
}

impl Journal {
    fn apply(&mut self, record: Record) {
        match record {
            Record::Fence { run, epoch } => {
                self.fences.insert(run, epoch);
            }
            Record::Started { run, call } => {
                self.calls.insert((run, call), Entry::Started);
            }
            Record::Finished { run, call, output } => {
                self.calls.insert((run, call), Entry::Finished(output));
            }
            Record::Reset { run, call } => {
                self.calls.remove(&(run, call));
            }
        }
    }

    /// Durably appends a record, then applies it.
    fn write(&mut self, record: Record) -> Result<(), MachineError> {
        if let Some(file) = &mut self.file {
            let mut line = serde_json::to_vec(&record).map_err(jerr)?;
            line.push(b'\n');
            file.write_all(&line).map_err(jerr)?;
            file.sync_data().map_err(jerr)?;
        }
        self.apply(record);
        Ok(())
    }

    /// Admits an invocation from `epoch`, raising the run's fence if needed.
    fn fence(&mut self, run: RunId, epoch: Epoch) -> Result<(), MachineError> {
        let current = self.fences.get(&run).copied().unwrap_or(Epoch(0));
        if epoch < current {
            return Err(MachineError::Fenced {
                presented: epoch,
                current,
            });
        }
        if epoch > current {
            self.write(Record::Fence { run, epoch })?;
        }
        Ok(())
    }
}

pub struct LocalMachine {
    workspace: PathBuf,
    worktrees: Option<PathBuf>,
    tools: Toolbox,
    journal: Mutex<Journal>,
    crash_before_next_finish: AtomicBool,
}

impl LocalMachine {
    /// A machine with an in-memory journal, for tests.
    pub fn in_memory(workspace: impl Into<PathBuf>, tools: Toolbox) -> Self {
        Self {
            workspace: workspace.into(),
            worktrees: None,
            tools,
            journal: Mutex::new(Journal::default()),
            crash_before_next_finish: AtomicBool::new(false),
        }
    }

    /// Where harness runs that work on a repository get their git
    /// worktrees. Without it, such runs are refused.
    pub fn with_worktrees(mut self, root: impl Into<PathBuf>) -> Self {
        self.worktrees = Some(root.into());
        self
    }

    pub fn worktrees_root(&self) -> Option<&Path> {
        self.worktrees.as_deref()
    }

    /// A machine whose journal is the append-only file at `journal_path`.
    /// A torn last line (from a crash mid-write) is ignored: that record
    /// was never acknowledged.
    pub fn open(
        workspace: impl Into<PathBuf>,
        journal_path: impl AsRef<Path>,
        tools: Toolbox,
    ) -> Result<Self, MachineError> {
        let path = journal_path.as_ref();
        let mut journal = Journal::default();
        if path.exists() {
            let reader = BufReader::new(File::open(path).map_err(jerr)?);
            let lines: Vec<String> = reader.lines().collect::<Result<_, _>>().map_err(jerr)?;
            let last = lines.len().saturating_sub(1);
            for (i, line) in lines.iter().enumerate() {
                match serde_json::from_str::<Record>(line) {
                    Ok(record) => journal.apply(record),
                    Err(_) if i == last => break,
                    Err(e) => return Err(MachineError::Journal(format!("line {}: {e}", i + 1))),
                }
            }
        }
        journal.file = Some(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(jerr)?,
        );
        Ok(Self {
            workspace: workspace.into(),
            worktrees: None,
            tools,
            journal: Mutex::new(journal),
            crash_before_next_finish: AtomicBool::new(false),
        })
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub fn toolbox(&self) -> &Toolbox {
        &self.tools
    }

    /// What the machine knows about a call. Also raises the run's fence to
    /// `epoch`, so after a new worker has asked, the old one is locked out.
    pub fn status(
        &self,
        run_id: RunId,
        epoch: Epoch,
        call_id: &CallId,
    ) -> Result<CallStatus, MachineError> {
        let mut j = self.lock()?;
        j.fence(run_id, epoch)?;
        Ok(match j.calls.get(&(run_id, call_id.clone())) {
            None => CallStatus::NotReceived,
            Some(Entry::Started) => CallStatus::Started,
            Some(Entry::Finished(out)) => CallStatus::Finished(out.clone()),
        })
    }

    /// Runs a call at most once: a call already finished returns its
    /// journaled output; a call that started but never finished is refused
    /// with `OutcomeUnknown`.
    pub async fn execute(&self, inv: Invocation<'_>) -> Result<ToolOutput, MachineError> {
        let tool = self
            .tools
            .get(inv.tool)
            .ok_or_else(|| MachineError::UnknownTool(inv.tool.to_owned()))?
            .clone();
        {
            let mut j = self.lock()?;
            j.fence(inv.run_id, inv.epoch)?;
            match j.calls.get(&(inv.run_id, inv.call_id.clone())) {
                Some(Entry::Finished(out)) => return Ok(out.clone()),
                Some(Entry::Started) => {
                    return Err(MachineError::OutcomeUnknown(inv.call_id.clone()));
                }
                None => j.write(Record::Started {
                    run: inv.run_id,
                    call: inv.call_id.clone(),
                })?,
            }
        }
        let output = tool.execute(inv.input, &self.workspace).await;
        if self.crash_before_next_finish.swap(false, Ordering::SeqCst) {
            return Err(MachineError::SimulatedCrash);
        }
        // No fence check here: the call was admitted under a valid epoch and
        // has run; its outcome must be recorded whatever happened since.
        self.lock()?.write(Record::Finished {
            run: inv.run_id,
            call: inv.call_id.clone(),
            output: output.clone(),
        })?;
        Ok(output)
    }

    /// Runs a pure or idempotent call again after it started but never
    /// finished. Never use this for side-effecting calls.
    pub async fn rerun(&self, inv: Invocation<'_>) -> Result<ToolOutput, MachineError> {
        {
            let mut j = self.lock()?;
            j.fence(inv.run_id, inv.epoch)?;
            if let Some(Entry::Started) = j.calls.get(&(inv.run_id, inv.call_id.clone())) {
                j.write(Record::Reset {
                    run: inv.run_id,
                    call: inv.call_id.clone(),
                })?;
            }
        }
        self.execute(inv).await
    }

    /// Fault injection for tests: the next call runs, but the machine
    /// "crashes" before journaling it as finished.
    #[doc(hidden)]
    pub fn crash_before_next_finish(&self) {
        self.crash_before_next_finish.store(true, Ordering::SeqCst);
    }

    fn lock(&self) -> Result<MutexGuard<'_, Journal>, MachineError> {
        self.journal
            .lock()
            .map_err(|_| MachineError::Journal("journal mutex poisoned".into()))
    }
}

fn jerr(e: impl std::fmt::Display) -> MachineError {
    MachineError::Journal(e.to_string())
}
