//! Storage layer for Shiioo.
//!
//! This module provides persistent storage abstractions for all Shiioo data,
//! including agents, events, blobs, and indexes.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                      STORAGE LAYER                              │
//! ├─────────────────────────────────────────────────────────────────┤
//! │                                                                 │
//! │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐             │
//! │  │ AgentStore  │  │  BlobStore  │  │  EventLog   │             │
//! │  │             │  │             │  │             │             │
//! │  │ • Agents    │  │ • Content   │  │ • Append    │             │
//! │  │ • Archetypes│  │   addressed │  │ • JSONL     │             │
//! │  │ • Status    │  │ • Immutable │  │ • Per-run   │             │
//! │  └─────────────┘  └─────────────┘  └─────────────┘             │
//! │                                                                 │
//! │  ┌─────────────┐  ┌─────────────┐                              │
//! │  │ IndexStore  │  │  Tenant     │                              │
//! │  │             │  │  Storage    │                              │
//! │  │ • Lookup    │  │             │                              │
//! │  │ • Range     │  │ • Isolation │                              │
//! │  │ • Composite │  │ • Quotas    │                              │
//! │  └─────────────┘  └─────────────┘                              │
//! │                                                                 │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Storage Types
//!
//! ## Agent Store
//!
//! The [`AgentStore`] trait provides CRUD operations for agents and archetypes.
//! Implementations include:
//! - [`InMemoryAgentStore`] - For testing and development
//! - [`RedbAgentStore`] - Persistent storage using redb
//!
//! ## Blob Store
//!
//! The [`BlobStore`] trait provides content-addressed storage for large objects.
//! Blobs are immutable and identified by their SHA-256 hash.
//!
//! ## Event Log
//!
//! The [`EventLogStore`] trait provides append-only event logging in JSONL format.
//! Events are organized by run ID for efficient replay.
//!
//! ## Index Store
//!
//! The [`IndexStore`] trait provides secondary indexes for efficient lookups.
//! Supports single-key, range, and composite queries.
//!
//! ## Tenant Storage
//!
//! The [`TenantStorage`] provides isolated storage per tenant with quota tracking.
//!
//! # Example
//!
//! ```rust,ignore
//! use shiioo_core::storage::{InMemoryAgentStore, AgentStore};
//! use shiioo_core::agent::{Agent, AgentId};
//!
//! let store = InMemoryAgentStore::new();
//!
//! // Store an agent
//! store.store_agent(&agent).await?;
//!
//! // Retrieve by ID
//! let agent = store.get_agent(&AgentId::new("agent-1")).await?;
//!
//! // List all agents
//! let agents = store.list_agents().await?;
//! ```

pub mod agent_store;
pub mod blob;
pub mod event_log;
pub mod index;
pub mod tenant_storage;

pub use agent_store::{AgentEvent, AgentStore, InMemoryAgentStore, RedbAgentStore};
pub use blob::{BlobStore, FilesystemBlobStore};
pub use event_log::{EventLogStore, JsonlEventLog};
pub use index::{IndexStore, RedbIndexStore};
pub use tenant_storage::{TenantStorage, TenantStorageStats};
