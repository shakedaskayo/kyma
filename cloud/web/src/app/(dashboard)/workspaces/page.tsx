'use client';
import { useEffect, useState } from 'react';
import Link from 'next/link';
import { api } from '@/lib/api';

export default function WorkspacesPage() {
  const [list, setList] = useState<any[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => { api.workspaces.list().then((r) => setList(r.workspaces)).catch((e) => setError(e.message)); }, []);
  if (error) return <div style={{ color: '#dc2626' }}>{error}</div>;
  if (!list) return <div>Loading…</div>;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-xl">Workspaces</h1>
        <Link
          href="/workspaces/new"
          className="h-9 px-3 rounded text-white text-sm flex items-center"
          style={{ background: 'var(--kyma-accent)' }}
        >
          New workspace
        </Link>
      </div>
      {list.length === 0 ? (
        <div className="text-sm" style={{ color: 'var(--kyma-muted)' }}>
          No workspaces yet. Create your first.
        </div>
      ) : (
        <ul className="space-y-2">
          {list.map((w) => (
            <li key={w.id} className="border rounded p-4" style={{ borderColor: 'var(--kyma-rule-soft)' }}>
              <Link href={`/workspaces/${w.slug}`} className="font-medium">{w.name}</Link>
              <div className="text-xs mt-1" style={{ color: 'var(--kyma-muted)' }}>
                {w.slug} · {w.kind} · {w.plan}
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
