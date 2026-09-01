import type { SchemaDoc } from "@pensieve-ai/client";

export const SCHEMA_FIXTURE: SchemaDoc = {
  databases: [
    {
      name: "production",
      tables: [
        {
          name: "events",
          columns: [
            { name: "ts",        type: "DateTime64(3)",  nullable: false },
            { name: "level",     type: "LowCardinality(String)", nullable: false },
            { name: "service",   type: "String",         nullable: false },
            { name: "trace_id",  type: "String",         nullable: true },
            { name: "span_id",   type: "String",         nullable: true },
            { name: "msg",       type: "String",         nullable: false },
            { name: "host",      type: "String",         nullable: false },
            { name: "env",       type: "LowCardinality(String)", nullable: false },
          ],
        },
        {
          name: "metrics",
          columns: [
            { name: "ts",        type: "DateTime64(3)",  nullable: false },
            { name: "metric",    type: "String",         nullable: false },
            { name: "value",     type: "Float64",        nullable: false },
            { name: "labels",    type: "Map(String, String)", nullable: false },
            { name: "host",      type: "String",         nullable: false },
          ],
        },
        {
          name: "traces",
          columns: [
            { name: "ts",           type: "DateTime64(3)",  nullable: false },
            { name: "trace_id",     type: "String",         nullable: false },
            { name: "span_id",      type: "String",         nullable: false },
            { name: "parent_span",  type: "String",         nullable: true },
            { name: "service",      type: "String",         nullable: false },
            { name: "operation",    type: "String",         nullable: false },
            { name: "duration_ms",  type: "Float64",        nullable: false },
            { name: "status",       type: "LowCardinality(String)", nullable: false },
          ],
        },
      ],
    },
  ],
};
