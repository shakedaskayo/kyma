CREATE EXTENSION IF NOT EXISTS "pgcrypto";

CREATE TABLE dashboards (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name TEXT NOT NULL,
  description TEXT,
  time_range_preset TEXT NOT NULL DEFAULT '1h',
  refresh_interval_seconds INTEGER,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE dashboard_panels (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  dashboard_id UUID NOT NULL REFERENCES dashboards(id) ON DELETE CASCADE,
  title TEXT NOT NULL,
  panel_type TEXT NOT NULL CHECK (panel_type IN ('chart','table','markdown','stat')),
  query TEXT,
  database_name TEXT,
  config JSONB NOT NULL DEFAULT '{}',
  grid_x INTEGER NOT NULL,
  grid_y INTEGER NOT NULL,
  grid_w INTEGER NOT NULL,
  grid_h INTEGER NOT NULL,
  display_order INTEGER NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX dashboard_panels_dashboard_id_idx ON dashboard_panels(dashboard_id, display_order);
