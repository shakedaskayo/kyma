'use client';
import { useEffect, useState } from 'react';
import { useParams } from 'next/navigation';
import Link from 'next/link';
import { api } from '@/lib/api';
import { McpInstallWidget } from '@/components/mcp-install-widget';

export default function WorkspaceDetailPage() {
  const params = useParams<{ slug: string }>();
  const slug = params.slug;
  const [workspace, setWorkspace] = useState<any>(null);
  const [tokens, setTokens] = useState<any[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([
      api.workspaces.get(slug),
      api.workspaces.listTokens(slug),
    ]).then(([{ workspace }, { tokens }]) => { setWorkspace(workspace); setTokens(tokens); })
      .catch((e) => setError(e.message));
  }, [slug]);

  if (error) return <div style={{ color: '#dc2626' }}>{error}</div>;
  if (!workspace) return <div>Loading…</div>;

  return (
    <div className="space-y-8">
      <div className="flex items-center justify-between">
        <h1 className="text-xl">{workspace.name}</h1>
        <Link href={`/workspaces/${slug}/billing`} className="text-sm underline">Billing</Link>
      </div>

      <McpInstallWidget slug={slug} mcpEndpoint={workspace.mcpEndpoint} />

      <section className="space-y-3">
        <h2 className="text-base">Tokens</h2>
        {tokens.length === 0 ? (
          <div className="text-sm" style={{ color: 'var(--kyma-muted)' }}>No tokens yet.</div>
        ) : (
          <ul className="space-y-2">
            {tokens.map((t) => (
              <li key={t.id} className="border rounded p-3 text-sm flex items-center justify-between"
                  style={{ borderColor: 'var(--kyma-rule-soft)' }}>
                <span>
                  <code className="font-mono">{t.prefix}…</code>{' '}
                  <span style={{ color: 'var(--kyma-muted)' }}>· {t.scopes}</span>
                </span>
                {t.revokedAt
                  ? <span style={{ color: 'var(--kyma-muted)' }}>revoked</span>
                  : <button
                      onClick={async () => { await api.workspaces.revokeToken(slug, t.id); location.reload(); }}
                      className="text-xs underline" style={{ color: '#dc2626' }}
                    >revoke</button>}
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
