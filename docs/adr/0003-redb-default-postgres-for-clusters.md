# 0003. redb by default, Postgres for clusters

- Status: Accepted
- Date: 2026-10-22

## Context

Shiioo must run on a laptop with no setup (`shiioo up`), and also as a cluster with many workers. The event log needs atomic multi-event appends, durable commits, and a per-run lock for the lease and `seq` checks.

Options considered:

| Option | For | Against |
|---|---|---|
| Postgres only | Mature, concurrent writers, queue and notifications built in | A server to install and run; wrong for on-device use |
| SQLite (C) | Ubiquitous, SQL | Not Rust; one writer |
| redb | Pure Rust, single file, ACID, crash-safe, stable file format (4.x), no server | Key-value only; one writer per file |
| Turso (Rust SQLite rewrite) | SQL, MVCC concurrent writes, async I/O | Beta (v0.7 as of 2026-07), MVCC still experimental |
| fjall, sled | Rust LSM engines | sled is unmaintained; fjall fits but adds compaction tuning with no gain over redb at our write rates |

## Decision

- Hide storage behind the `EventStore` trait. The rules live in `shiioo-core`; backends only persist.
- **redb is the default backend**, for on-device and single-node deployments.
- **Postgres is the cluster backend**, behind the `postgres` feature. It also provides the run queue (`SKIP LOCKED`) and wakeups (`LISTEN/NOTIFY`), so clusters need no Redis.
- All backends pass the same conformance suite, including concurrent appends and reopen-after-crash tests.

## Consequences

- No database to install for the common case.
- Two real backends to maintain. The shared rules and the conformance suite keep that cost low.
- redb serializes all writes in one file. At agent speeds (a few events per second per run) one node handles hundreds of active runs; beyond that, use Postgres.
- Features that need queries across runs (search, analytics) are built as projections, not as backend-specific SQL.

## Revisit when

Turso ships a stable release with production-ready concurrent writes. It could then replace redb and give one embedded engine with SQL and multiple writers.
