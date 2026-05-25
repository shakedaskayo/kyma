-- Graph registrations: bind a node table + edge table (with column roles)
-- in a database to a named property-graph. Tables themselves are ordinary
-- kyma tables; this is metadata only.
CREATE TABLE graph_registrations (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id   uuid NOT NULL,
    database    text NOT NULL,
    name        text NOT NULL,
    node_table  text NOT NULL,
    edge_table  text NOT NULL,
    id_col      text NOT NULL,
    label_col   text NOT NULL,
    src_col     text NOT NULL,
    dst_col     text NOT NULL,
    type_col    text NOT NULL,
    realm_col   text,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, database, name)
);
CREATE INDEX graph_registrations_tenant_idx ON graph_registrations (tenant_id);
