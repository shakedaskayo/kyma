--
-- PostgreSQL database dump
--

\restrict 2BIs78zm0HnBAkuGqPPqj2RiXVOFwSmobrdAq5mmZ82IyQcXaGlJR3dG8gJudam

-- Dumped from database version 16.14 (Debian 16.14-1.pgdg12+1)
-- Dumped by pg_dump version 16.14 (Homebrew)

SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;
SET row_security = off;

--
-- Name: btree_gist; Type: EXTENSION; Schema: -; Owner: -
--

CREATE EXTENSION IF NOT EXISTS btree_gist WITH SCHEMA public;


--
-- Name: EXTENSION btree_gist; Type: COMMENT; Schema: -; Owner: -
--

COMMENT ON EXTENSION btree_gist IS 'support for indexing common datatypes in GiST';


--
-- Name: pgcrypto; Type: EXTENSION; Schema: -; Owner: -
--

CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;


--
-- Name: EXTENSION pgcrypto; Type: COMMENT; Schema: -; Owner: -
--

COMMENT ON EXTENSION pgcrypto IS 'cryptographic functions';


--
-- Name: uuid-ossp; Type: EXTENSION; Schema: -; Owner: -
--

CREATE EXTENSION IF NOT EXISTS "uuid-ossp" WITH SCHEMA public;


--
-- Name: EXTENSION "uuid-ossp"; Type: COMMENT; Schema: -; Owner: -
--

COMMENT ON EXTENSION "uuid-ossp" IS 'generate universally unique identifiers (UUIDs)';


--
-- Name: vector; Type: EXTENSION; Schema: -; Owner: -
--

CREATE EXTENSION IF NOT EXISTS vector WITH SCHEMA public;


--
-- Name: EXTENSION vector; Type: COMMENT; Schema: -; Owner: -
--

COMMENT ON EXTENSION vector IS 'vector data type and ivfflat and hnsw access methods';


SET default_tablespace = '';

SET default_table_access_method = heap;

--
-- Name: _sqlx_migrations; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public._sqlx_migrations (
    version bigint NOT NULL,
    description text NOT NULL,
    installed_on timestamp with time zone DEFAULT now() NOT NULL,
    success boolean NOT NULL,
    checksum bytea NOT NULL,
    execution_time bigint NOT NULL
);


--
-- Name: agent_replay_cache; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.agent_replay_cache (
    cache_key bytea NOT NULL,
    layer text NOT NULL,
    response_json jsonb NOT NULL,
    model_id text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    hit_count integer DEFAULT 0 NOT NULL,
    tenant_id uuid NOT NULL,
    CONSTRAINT agent_replay_cache_layer_check CHECK ((layer = ANY (ARRAY['generate'::text, 'run'::text])))
);


--
-- Name: agent_runs; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.agent_runs (
    run_id uuid NOT NULL,
    question text NOT NULL,
    model_id text NOT NULL,
    auth_subject text NOT NULL,
    session_id uuid,
    started_at timestamp with time zone NOT NULL,
    finished_at timestamp with time zone NOT NULL,
    status text NOT NULL,
    usage_json jsonb NOT NULL,
    trace_json jsonb NOT NULL,
    replay_cache_hit boolean DEFAULT false NOT NULL,
    tenant_id uuid NOT NULL,
    CONSTRAINT agent_runs_status_check CHECK ((status = ANY (ARRAY['success'::text, 'error'::text, 'budget_exceeded'::text, 'cancelled'::text, 'replay_miss'::text])))
);


--
-- Name: agent_session_turns; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.agent_session_turns (
    session_id uuid NOT NULL,
    turn_index integer NOT NULL,
    role text NOT NULL,
    content_json jsonb NOT NULL,
    run_id uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    tenant_id uuid NOT NULL,
    CONSTRAINT agent_session_turns_role_check CHECK ((role = ANY (ARRAY['user'::text, 'assistant'::text])))
);


--
-- Name: agent_sessions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.agent_sessions (
    session_id uuid NOT NULL,
    auth_subject text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    last_active timestamp with time zone DEFAULT now() NOT NULL,
    metadata_json jsonb DEFAULT '{}'::jsonb NOT NULL,
    tenant_id uuid NOT NULL
);


--
-- Name: background_tasks; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.background_tasks (
    id uuid DEFAULT public.uuid_generate_v4() NOT NULL,
    kind text NOT NULL,
    table_id uuid,
    payload jsonb DEFAULT '{}'::jsonb NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    claimed_by uuid,
    claim_expires_at timestamp with time zone,
    priority integer DEFAULT 0 NOT NULL,
    attempt integer DEFAULT 0 NOT NULL,
    max_attempts integer DEFAULT 3 NOT NULL,
    last_error text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    tenant_id uuid NOT NULL,
    CONSTRAINT background_tasks_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'claimed'::text, 'done'::text, 'failed'::text])))
);


--
-- Name: column_metadata; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.column_metadata (
    database text NOT NULL,
    table_name text NOT NULL,
    column_name text NOT NULL,
    column_type text NOT NULL,
    description text,
    embedding_model_id text,
    dimension integer,
    distance_metric text,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    tenant_id uuid NOT NULL
);


--
-- Name: connector_cursors; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.connector_cursors (
    connector_id uuid NOT NULL,
    cursor_jsonb jsonb,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    tenant_id uuid NOT NULL
);


--
-- Name: connector_leases; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.connector_leases (
    connector_id uuid NOT NULL,
    node_id text NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    tenant_id uuid NOT NULL
);


--
-- Name: connectors; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.connectors (
    id uuid DEFAULT public.uuid_generate_v4() NOT NULL,
    name text NOT NULL,
    type text NOT NULL,
    target_database text NOT NULL,
    target_table text NOT NULL,
    config_jsonb jsonb NOT NULL,
    schedule_ms bigint NOT NULL,
    drive_model text NOT NULL,
    enabled boolean DEFAULT true NOT NULL,
    disabled_reason text,
    last_run_at timestamp with time zone,
    last_success_at timestamp with time zone,
    last_error text,
    last_rows_ingested bigint,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    tenant_id uuid NOT NULL,
    kms_key_id text,
    encrypted_secrets bytea,
    CONSTRAINT connectors_drive_model_check CHECK ((drive_model = ANY (ARRAY['periodic'::text, 'continuous'::text]))),
    CONSTRAINT connectors_schedule_ms_check CHECK (((schedule_ms >= 100) AND (schedule_ms <= 86400000)))
);


--
-- Name: dashboard_panels; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.dashboard_panels (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    dashboard_id uuid NOT NULL,
    title text NOT NULL,
    panel_type text NOT NULL,
    query text,
    database_name text,
    config jsonb DEFAULT '{}'::jsonb NOT NULL,
    grid_x integer NOT NULL,
    grid_y integer NOT NULL,
    grid_w integer NOT NULL,
    grid_h integer NOT NULL,
    display_order integer NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    tenant_id uuid NOT NULL,
    CONSTRAINT dashboard_panels_panel_type_check CHECK ((panel_type = ANY (ARRAY['chart'::text, 'table'::text, 'markdown'::text, 'stat'::text])))
);


--
-- Name: dashboards; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.dashboards (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    name text NOT NULL,
    description text,
    time_range_preset text DEFAULT '1h'::text NOT NULL,
    refresh_interval_seconds integer,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    tenant_id uuid NOT NULL
);


--
-- Name: databases; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.databases (
    id uuid DEFAULT public.uuid_generate_v4() NOT NULL,
    name text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    tenant_id uuid NOT NULL
);


--
-- Name: extents; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.extents (
    id uuid DEFAULT public.uuid_generate_v4() NOT NULL,
    table_id uuid NOT NULL,
    manifest_id uuid,
    schema_snapshot_id uuid NOT NULL,
    object_path text NOT NULL,
    byte_size bigint NOT NULL,
    row_count bigint NOT NULL,
    min_timestamp timestamp with time zone,
    max_timestamp timestamp with time zone,
    column_stats jsonb DEFAULT '{}'::jsonb NOT NULL,
    present_paths text[] DEFAULT '{}'::text[] NOT NULL,
    bloom_filters bytea,
    compaction_gen integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    deleted_at timestamp with time zone,
    tenant_id uuid NOT NULL
);


--
-- Name: ingest_ledger; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.ingest_ledger (
    idempotency_key text NOT NULL,
    table_id uuid NOT NULL,
    snapshot_id uuid NOT NULL,
    rows_ingested bigint NOT NULL,
    bytes_written bigint NOT NULL,
    applied_at timestamp with time zone DEFAULT now() NOT NULL,
    ttl_expires_at timestamp with time zone NOT NULL,
    tenant_id uuid NOT NULL
);


--
-- Name: manifests; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.manifests (
    id uuid DEFAULT public.uuid_generate_v4() NOT NULL,
    snapshot_id uuid NOT NULL,
    kind text NOT NULL,
    extent_count integer DEFAULT 0 NOT NULL,
    byte_size bigint DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    tenant_id uuid NOT NULL,
    CONSTRAINT manifests_kind_check CHECK ((kind = ANY (ARRAY['data'::text, 'delete'::text, 'compaction'::text])))
);


--
-- Name: nodes; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.nodes (
    id uuid DEFAULT public.uuid_generate_v4() NOT NULL,
    role text NOT NULL,
    endpoint text NOT NULL,
    capabilities jsonb DEFAULT '{}'::jsonb NOT NULL,
    lease_id uuid NOT NULL,
    last_heartbeat timestamp with time zone DEFAULT now() NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT nodes_role_check CHECK ((role = ANY (ARRAY['all_in_one'::text, 'ingest'::text, 'query'::text, 'compaction'::text])))
);


--
-- Name: schema_embeddings; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.schema_embeddings (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    database text NOT NULL,
    table_name text NOT NULL,
    column_name text,
    kind text NOT NULL,
    text_source text NOT NULL,
    text_source_sha256 bytea NOT NULL,
    text_format_version text DEFAULT 'v1'::text NOT NULL,
    model_id text NOT NULL,
    embedding public.vector(384) NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    tenant_id uuid NOT NULL,
    CONSTRAINT schema_embeddings_kind_check CHECK ((kind = ANY (ARRAY['table'::text, 'column'::text])))
);


--
-- Name: schema_snapshots; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.schema_snapshots (
    id uuid DEFAULT public.uuid_generate_v4() NOT NULL,
    table_id uuid NOT NULL,
    arrow_schema jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    tenant_id uuid NOT NULL
);


--
-- Name: snapshots; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.snapshots (
    id uuid DEFAULT public.uuid_generate_v4() NOT NULL,
    table_id uuid NOT NULL,
    parent_id uuid,
    sequence_number bigint NOT NULL,
    schema_snapshot_id uuid NOT NULL,
    summary jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    tenant_id uuid NOT NULL
);


--
-- Name: tables; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.tables (
    id uuid DEFAULT public.uuid_generate_v4() NOT NULL,
    database_id uuid NOT NULL,
    name text NOT NULL,
    current_snapshot_id uuid,
    schema_snapshot_id uuid,
    config jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    tenant_id uuid NOT NULL
);


--
-- Name: _sqlx_migrations _sqlx_migrations_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public._sqlx_migrations
    ADD CONSTRAINT _sqlx_migrations_pkey PRIMARY KEY (version);


--
-- Name: agent_replay_cache agent_replay_cache_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.agent_replay_cache
    ADD CONSTRAINT agent_replay_cache_pkey PRIMARY KEY (tenant_id, cache_key);


--
-- Name: agent_runs agent_runs_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.agent_runs
    ADD CONSTRAINT agent_runs_pkey PRIMARY KEY (run_id);


--
-- Name: agent_session_turns agent_session_turns_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.agent_session_turns
    ADD CONSTRAINT agent_session_turns_pkey PRIMARY KEY (session_id, turn_index);


--
-- Name: agent_sessions agent_sessions_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.agent_sessions
    ADD CONSTRAINT agent_sessions_pkey PRIMARY KEY (session_id);


--
-- Name: background_tasks background_tasks_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.background_tasks
    ADD CONSTRAINT background_tasks_pkey PRIMARY KEY (id);


--
-- Name: column_metadata column_metadata_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.column_metadata
    ADD CONSTRAINT column_metadata_pkey PRIMARY KEY (tenant_id, database, table_name, column_name);


--
-- Name: connector_cursors connector_cursors_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.connector_cursors
    ADD CONSTRAINT connector_cursors_pkey PRIMARY KEY (connector_id);


--
-- Name: connector_leases connector_leases_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.connector_leases
    ADD CONSTRAINT connector_leases_pkey PRIMARY KEY (connector_id);


--
-- Name: connectors connectors_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.connectors
    ADD CONSTRAINT connectors_pkey PRIMARY KEY (id);


--
-- Name: connectors connectors_tenant_name_uniq; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.connectors
    ADD CONSTRAINT connectors_tenant_name_uniq UNIQUE (tenant_id, name);


--
-- Name: dashboard_panels dashboard_panels_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.dashboard_panels
    ADD CONSTRAINT dashboard_panels_pkey PRIMARY KEY (id);


--
-- Name: dashboards dashboards_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.dashboards
    ADD CONSTRAINT dashboards_pkey PRIMARY KEY (id);


--
-- Name: databases databases_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.databases
    ADD CONSTRAINT databases_pkey PRIMARY KEY (id);


--
-- Name: databases databases_tenant_name_uniq; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.databases
    ADD CONSTRAINT databases_tenant_name_uniq UNIQUE (tenant_id, name);


--
-- Name: extents extents_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.extents
    ADD CONSTRAINT extents_pkey PRIMARY KEY (id);


--
-- Name: ingest_ledger ingest_ledger_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.ingest_ledger
    ADD CONSTRAINT ingest_ledger_pkey PRIMARY KEY (tenant_id, idempotency_key);


--
-- Name: manifests manifests_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.manifests
    ADD CONSTRAINT manifests_pkey PRIMARY KEY (id);


--
-- Name: nodes nodes_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.nodes
    ADD CONSTRAINT nodes_pkey PRIMARY KEY (id);


--
-- Name: schema_embeddings schema_embeddings_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.schema_embeddings
    ADD CONSTRAINT schema_embeddings_pkey PRIMARY KEY (id);


--
-- Name: schema_snapshots schema_snapshots_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.schema_snapshots
    ADD CONSTRAINT schema_snapshots_pkey PRIMARY KEY (id);


--
-- Name: snapshots snapshots_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.snapshots
    ADD CONSTRAINT snapshots_pkey PRIMARY KEY (id);


--
-- Name: snapshots snapshots_table_id_sequence_number_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.snapshots
    ADD CONSTRAINT snapshots_table_id_sequence_number_key UNIQUE (table_id, sequence_number);


--
-- Name: tables tables_database_id_name_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tables
    ADD CONSTRAINT tables_database_id_name_key UNIQUE (database_id, name);


--
-- Name: tables tables_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tables
    ADD CONSTRAINT tables_pkey PRIMARY KEY (id);


--
-- Name: agent_replay_cache_layer_model; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX agent_replay_cache_layer_model ON public.agent_replay_cache USING btree (layer, model_id);


--
-- Name: agent_runs_session; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX agent_runs_session ON public.agent_runs USING btree (session_id, started_at DESC);


--
-- Name: agent_runs_subject_time; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX agent_runs_subject_time ON public.agent_runs USING btree (auth_subject, started_at DESC);


--
-- Name: agent_runs_tenant_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX agent_runs_tenant_idx ON public.agent_runs USING btree (tenant_id, started_at DESC);


--
-- Name: agent_session_turns_tenant_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX agent_session_turns_tenant_idx ON public.agent_session_turns USING btree (tenant_id);


--
-- Name: agent_sessions_tenant_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX agent_sessions_tenant_idx ON public.agent_sessions USING btree (tenant_id);


--
-- Name: background_tasks_claimed_expiry; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX background_tasks_claimed_expiry ON public.background_tasks USING btree (claim_expires_at) WHERE (status = 'claimed'::text);


--
-- Name: background_tasks_connector_tick_uniq; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX background_tasks_connector_tick_uniq ON public.background_tasks USING btree (((payload ->> 'connector_id'::text)), ((payload ->> 'scheduled_for'::text))) WHERE ((kind = 'connector_tick'::text) AND (status = ANY (ARRAY['pending'::text, 'claimed'::text])));


--
-- Name: background_tasks_pending; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX background_tasks_pending ON public.background_tasks USING btree (kind, priority DESC, created_at) WHERE (status = 'pending'::text);


--
-- Name: background_tasks_tenant_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX background_tasks_tenant_idx ON public.background_tasks USING btree (tenant_id);


--
-- Name: connector_cursors_tenant_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX connector_cursors_tenant_idx ON public.connector_cursors USING btree (tenant_id);


--
-- Name: connector_leases_tenant_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX connector_leases_tenant_idx ON public.connector_leases USING btree (tenant_id);


--
-- Name: connectors_enabled_drive_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX connectors_enabled_drive_idx ON public.connectors USING btree (drive_model, enabled) WHERE (enabled = true);


--
-- Name: connectors_tenant_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX connectors_tenant_idx ON public.connectors USING btree (tenant_id);


--
-- Name: dashboard_panels_dashboard_id_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX dashboard_panels_dashboard_id_idx ON public.dashboard_panels USING btree (dashboard_id, display_order);


--
-- Name: dashboard_panels_tenant_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX dashboard_panels_tenant_idx ON public.dashboard_panels USING btree (tenant_id);


--
-- Name: dashboards_tenant_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX dashboards_tenant_idx ON public.dashboards USING btree (tenant_id);


--
-- Name: databases_tenant_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX databases_tenant_idx ON public.databases USING btree (tenant_id);


--
-- Name: extents_live; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX extents_live ON public.extents USING btree (tenant_id, table_id) WHERE (deleted_at IS NULL);


--
-- Name: extents_present_paths; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX extents_present_paths ON public.extents USING gin (present_paths);


--
-- Name: extents_tbl_ts_range; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX extents_tbl_ts_range ON public.extents USING gist (table_id, tstzrange(min_timestamp, max_timestamp));


--
-- Name: extents_tenant_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX extents_tenant_idx ON public.extents USING btree (tenant_id);


--
-- Name: ingest_ledger_ttl; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ingest_ledger_ttl ON public.ingest_ledger USING btree (ttl_expires_at);


--
-- Name: manifests_tenant_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX manifests_tenant_idx ON public.manifests USING btree (tenant_id);


--
-- Name: nodes_last_heartbeat; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX nodes_last_heartbeat ON public.nodes USING btree (last_heartbeat);


--
-- Name: schema_embeddings_db; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX schema_embeddings_db ON public.schema_embeddings USING btree (tenant_id, database);


--
-- Name: schema_embeddings_hnsw; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX schema_embeddings_hnsw ON public.schema_embeddings USING hnsw (embedding public.vector_cosine_ops);


--
-- Name: schema_embeddings_uniq_column; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX schema_embeddings_uniq_column ON public.schema_embeddings USING btree (tenant_id, database, table_name, column_name, model_id) WHERE (column_name IS NOT NULL);


--
-- Name: schema_embeddings_uniq_table; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX schema_embeddings_uniq_table ON public.schema_embeddings USING btree (tenant_id, database, table_name, model_id) WHERE (column_name IS NULL);


--
-- Name: schema_snapshots_tenant_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX schema_snapshots_tenant_idx ON public.schema_snapshots USING btree (tenant_id);


--
-- Name: snapshots_tenant_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX snapshots_tenant_idx ON public.snapshots USING btree (tenant_id);


--
-- Name: tables_tenant_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX tables_tenant_idx ON public.tables USING btree (tenant_id);


--
-- Name: agent_session_turns agent_session_turns_run_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.agent_session_turns
    ADD CONSTRAINT agent_session_turns_run_id_fkey FOREIGN KEY (run_id) REFERENCES public.agent_runs(run_id);


--
-- Name: agent_session_turns agent_session_turns_session_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.agent_session_turns
    ADD CONSTRAINT agent_session_turns_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.agent_sessions(session_id) ON DELETE CASCADE;


--
-- Name: background_tasks background_tasks_claimed_by_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.background_tasks
    ADD CONSTRAINT background_tasks_claimed_by_fkey FOREIGN KEY (claimed_by) REFERENCES public.nodes(id) ON DELETE SET NULL;


--
-- Name: background_tasks background_tasks_table_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.background_tasks
    ADD CONSTRAINT background_tasks_table_id_fkey FOREIGN KEY (table_id) REFERENCES public.tables(id) ON DELETE CASCADE;


--
-- Name: connector_cursors connector_cursors_connector_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.connector_cursors
    ADD CONSTRAINT connector_cursors_connector_id_fkey FOREIGN KEY (connector_id) REFERENCES public.connectors(id) ON DELETE CASCADE;


--
-- Name: connector_leases connector_leases_connector_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.connector_leases
    ADD CONSTRAINT connector_leases_connector_id_fkey FOREIGN KEY (connector_id) REFERENCES public.connectors(id) ON DELETE CASCADE;


--
-- Name: dashboard_panels dashboard_panels_dashboard_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.dashboard_panels
    ADD CONSTRAINT dashboard_panels_dashboard_id_fkey FOREIGN KEY (dashboard_id) REFERENCES public.dashboards(id) ON DELETE CASCADE;


--
-- Name: extents extents_manifest_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.extents
    ADD CONSTRAINT extents_manifest_id_fkey FOREIGN KEY (manifest_id) REFERENCES public.manifests(id) ON DELETE CASCADE;


--
-- Name: extents extents_schema_snapshot_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.extents
    ADD CONSTRAINT extents_schema_snapshot_id_fkey FOREIGN KEY (schema_snapshot_id) REFERENCES public.schema_snapshots(id);


--
-- Name: extents extents_table_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.extents
    ADD CONSTRAINT extents_table_id_fkey FOREIGN KEY (table_id) REFERENCES public.tables(id) ON DELETE CASCADE;


--
-- Name: ingest_ledger ingest_ledger_snapshot_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.ingest_ledger
    ADD CONSTRAINT ingest_ledger_snapshot_id_fkey FOREIGN KEY (snapshot_id) REFERENCES public.snapshots(id) ON DELETE CASCADE;


--
-- Name: ingest_ledger ingest_ledger_table_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.ingest_ledger
    ADD CONSTRAINT ingest_ledger_table_id_fkey FOREIGN KEY (table_id) REFERENCES public.tables(id) ON DELETE CASCADE;


--
-- Name: manifests manifests_snapshot_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.manifests
    ADD CONSTRAINT manifests_snapshot_id_fkey FOREIGN KEY (snapshot_id) REFERENCES public.snapshots(id) ON DELETE CASCADE;


--
-- Name: schema_snapshots schema_snapshots_table_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.schema_snapshots
    ADD CONSTRAINT schema_snapshots_table_fk FOREIGN KEY (table_id) REFERENCES public.tables(id) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED;


--
-- Name: snapshots snapshots_parent_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.snapshots
    ADD CONSTRAINT snapshots_parent_id_fkey FOREIGN KEY (parent_id) REFERENCES public.snapshots(id);


--
-- Name: snapshots snapshots_schema_snapshot_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.snapshots
    ADD CONSTRAINT snapshots_schema_snapshot_id_fkey FOREIGN KEY (schema_snapshot_id) REFERENCES public.schema_snapshots(id);


--
-- Name: snapshots snapshots_table_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.snapshots
    ADD CONSTRAINT snapshots_table_id_fkey FOREIGN KEY (table_id) REFERENCES public.tables(id) ON DELETE CASCADE;


--
-- Name: tables tables_current_snapshot_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tables
    ADD CONSTRAINT tables_current_snapshot_fk FOREIGN KEY (current_snapshot_id) REFERENCES public.snapshots(id) DEFERRABLE INITIALLY DEFERRED;


--
-- Name: tables tables_database_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tables
    ADD CONSTRAINT tables_database_id_fkey FOREIGN KEY (database_id) REFERENCES public.databases(id) ON DELETE CASCADE;


--
-- Name: tables tables_schema_snapshot_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tables
    ADD CONSTRAINT tables_schema_snapshot_id_fkey FOREIGN KEY (schema_snapshot_id) REFERENCES public.schema_snapshots(id);


--
-- PostgreSQL database dump complete
--

\unrestrict 2BIs78zm0HnBAkuGqPPqj2RiXVOFwSmobrdAq5mmZ82IyQcXaGlJR3dG8gJudam

