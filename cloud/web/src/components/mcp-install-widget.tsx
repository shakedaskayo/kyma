'use client';
import { useState } from 'react';
import { api } from '@/lib/api';

export function McpInstallWidget({ slug, mcpEndpoint }: { slug: string; mcpEndpoint: string }) {
  const [token, setToken] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function mint() {
    setPending(true); setError(null);
    try { setToken((await api.workspaces.mintToken(slug, 'claude-skill')).token); }
    catch (err: any) { setError(err.message); }
    finally { setPending(false); }
  }

  return (
    <section className="border rounded p-4 space-y-3" style={{ borderColor: 'var(--kyma-rule-soft)' }}>
      <h2 className="text-base">Connect Claude</h2>
      <ol className="list-decimal pl-5 text-sm space-y-2" style={{ color: 'var(--kyma-fg-soft)' }}>
        <li>Install the kyma Claude skill: <code>/skill install kyma</code> in Claude.</li>
        <li>Mint a workspace token below — copy it once; we never show it again.</li>
        <li>Paste the URL and token into the skill prompt and start asking questions.</li>
      </ol>

      <div className="space-y-2">
        <label className="text-xs uppercase tracking-wide" style={{ color: 'var(--kyma-muted)' }}>
          MCP endpoint
        </label>
        <input
          readOnly value={mcpEndpoint}
          onClick={(e) => (e.target as HTMLInputElement).select()}
          className="w-full h-10 px-3 rounded border font-mono text-sm bg-transparent"
          style={{ borderColor: 'var(--kyma-rule-soft)' }}
        />
      </div>

      {token ? (
        <div className="space-y-2">
          <label className="text-xs uppercase tracking-wide" style={{ color: 'var(--kyma-muted)' }}>
            Token (copy now — won't be shown again)
          </label>
          <input
            readOnly value={token}
            onClick={(e) => (e.target as HTMLInputElement).select()}
            className="w-full h-10 px-3 rounded border font-mono text-sm bg-transparent"
            style={{ borderColor: 'var(--kyma-rule-soft)' }}
          />
          <pre
            className="text-xs p-3 rounded overflow-x-auto"
            style={{ background: 'var(--kyma-bg-soft)' }}
          >{`{
  "mcpServers": {
    "kyma": {
      "transport": "http",
      "url": "${mcpEndpoint}",
      "headers": { "Authorization": "Bearer ${token}" }
    }
  }
}`}</pre>
        </div>
      ) : (
        <button
          onClick={mint} disabled={pending}
          className="h-10 px-4 rounded text-white text-sm"
          style={{ background: 'var(--kyma-accent)' }}
        >
          {pending ? 'Minting…' : 'Mint MCP token'}
        </button>
      )}
      {error && <div className="text-sm" style={{ color: '#dc2626' }}>{error}</div>}
    </section>
  );
}
