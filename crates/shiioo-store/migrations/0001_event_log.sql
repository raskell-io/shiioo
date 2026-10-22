-- One row per run: the head that every write is checked against. Writers
-- lock this row (SELECT ... FOR UPDATE), so writes to one run are serialized
-- while different runs proceed in parallel.
CREATE TABLE runs (
    run_id              uuid        PRIMARY KEY,
    next_seq            bigint      NOT NULL CHECK (next_seq >= 0),
    last_hash           bytea       CHECK (octet_length(last_hash) = 32),
    closed              boolean     NOT NULL DEFAULT false,
    lease_worker        text,
    lease_epoch         bigint      NOT NULL DEFAULT 0 CHECK (lease_epoch >= 0),
    lease_expires_at_ms bigint,
    created_at          timestamptz NOT NULL DEFAULT now(),
    CHECK ((lease_epoch = 0) = (lease_worker IS NULL)),
    CHECK ((lease_epoch = 0) = (lease_expires_at_ms IS NULL))
);

CREATE TABLE events (
    run_id         uuid     NOT NULL REFERENCES runs (run_id),
    seq            bigint   NOT NULL CHECK (seq >= 0),
    epoch          bigint   NOT NULL CHECK (epoch >= 0),
    recorded_at_ms bigint   NOT NULL,
    schema         smallint NOT NULL,
    prev_hash      bytea    CHECK (octet_length(prev_hash) = 32),
    hash           bytea    NOT NULL CHECK (octet_length(hash) = 32),
    -- `json`, not `jsonb`: keeps the exact text that was hashed.
    payload        json     NOT NULL,
    PRIMARY KEY (run_id, seq),
    CHECK ((seq = 0) = (prev_hash IS NULL))
);

-- The log is append-only, even for someone with a SQL prompt. Archiving
-- finished runs to cold storage will get its own, explicit path.
CREATE FUNCTION shiioo_events_append_only() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'shiioo events are append-only (% on events)', TG_OP;
END
$$;

CREATE TRIGGER events_no_update_or_delete
    BEFORE UPDATE OR DELETE ON events
    FOR EACH ROW EXECUTE FUNCTION shiioo_events_append_only();

CREATE TRIGGER events_no_truncate
    BEFORE TRUNCATE ON events
    FOR EACH STATEMENT EXECUTE FUNCTION shiioo_events_append_only();
