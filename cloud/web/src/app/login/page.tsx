'use client';
import { useState } from 'react';
import { api } from '@/lib/api';

export default function LoginPage() {
  const [email, setEmail] = useState('');
  const [sent, setSent] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError(null); setPending(true);
    try { await api.requestMagicLink(email); setSent(true); }
    catch (err: any) { setError(err.message); }
    finally { setPending(false); }
  }

  return (
    <main className="min-h-screen flex items-center justify-center px-4">
      <div className="w-full max-w-sm space-y-6">
        <div>
          <h1 className="text-2xl">kyma cloud</h1>
          <p className="text-sm" style={{ color: 'var(--kyma-muted)' }}>
            Sign in to your workspace.
          </p>
        </div>

        <button
          onClick={() => api.startGithubAuth()}
          className="w-full h-10 rounded border font-medium"
          style={{ borderColor: 'var(--kyma-rule-soft)', background: 'var(--kyma-bg-soft)' }}
        >
          Continue with GitHub
        </button>

        <div className="flex items-center gap-3 text-xs" style={{ color: 'var(--kyma-muted)' }}>
          <div className="flex-1 h-px" style={{ background: 'var(--kyma-rule-soft)' }} />
          OR
          <div className="flex-1 h-px" style={{ background: 'var(--kyma-rule-soft)' }} />
        </div>

        {sent ? (
          <div className="text-sm">Check your inbox — we sent a magic link to <strong>{email}</strong>.</div>
        ) : (
          <form onSubmit={submit} className="space-y-3">
            <input
              type="email" required value={email} onChange={(e) => setEmail(e.target.value)}
              placeholder="you@example.com"
              className="w-full h-10 px-3 rounded border bg-transparent"
              style={{ borderColor: 'var(--kyma-rule-soft)' }}
            />
            <button
              type="submit" disabled={pending}
              className="w-full h-10 rounded font-medium text-white"
              style={{ background: 'var(--kyma-accent)' }}
            >
              {pending ? 'Sending…' : 'Email me a link'}
            </button>
            {error && <div className="text-sm" style={{ color: '#dc2626' }}>{error}</div>}
          </form>
        )}
      </div>
    </main>
  );
}
