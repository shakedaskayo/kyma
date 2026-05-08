'use client';
import {
  AreaChart, Area, XAxis, YAxis, Tooltip, ResponsiveContainer,
} from 'recharts';

interface DayRow { day: string; mcpCalls: number; ingestBytes: number }

export function UsageChart({ rows }: { rows: DayRow[] }) {
  const data = rows.map((r) => ({
    day: r.day.slice(5),          // "MM-DD"
    calls: r.mcpCalls,
    mb: +(r.ingestBytes / 1024 / 1024).toFixed(1),
  }));

  return (
    <div className="space-y-4">
      <h3 className="text-sm font-medium">MCP calls – last 30 days</h3>
      <ResponsiveContainer width="100%" height={160}>
        <AreaChart data={data}>
          <XAxis dataKey="day" tick={{ fontSize: 11 }} />
          <YAxis tick={{ fontSize: 11 }} />
          <Tooltip />
          <Area type="monotone" dataKey="calls" stroke="var(--kyma-accent)"
                fill="var(--kyma-accent)" fillOpacity={0.15} />
        </AreaChart>
      </ResponsiveContainer>

      <h3 className="text-sm font-medium">Ingest (MiB) – last 30 days</h3>
      <ResponsiveContainer width="100%" height={160}>
        <AreaChart data={data}>
          <XAxis dataKey="day" tick={{ fontSize: 11 }} />
          <YAxis tick={{ fontSize: 11 }} />
          <Tooltip />
          <Area type="monotone" dataKey="mb" stroke="#8b5cf6"
                fill="#8b5cf6" fillOpacity={0.15} />
        </AreaChart>
      </ResponsiveContainer>
    </div>
  );
}
