'use client';
import { useState } from 'react';
import { useRouter } from 'next/navigation';
import { api } from '@/lib/api';

export default function NewWorkspacePage() {
  const router = useRouter();
  const [name, setName] = useState('');
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setPending(true); setError(null);
    try {
      const { workspace } = await api.workspaces.create(name);
      router.replace(`/workspaces/${workspace.slug}`);
    } catch (err: any) { setError(err.message); setPending(false); }
  }

  return (
    <form onSubmit={submit} className="max-w-md space-y-4">
      <h1 className="text-xl">New workspace</h1>
      <input
        required value={name} onChange={(e) => setName(e.target.value)}
        placeholder="My workspace"
        className="w-full h-10 px-3 rounded border bg-transparent"
        style={{ borderColor: 'var(--kyma-rule-soft)' }}
      />
      <button
        type="submit" disabled={pending}
        className="h-10 px-4 rounded text-white"
        style={{ background: 'var(--kyma-accent)' }}
      >
        {pending ? 'Creating…' : 'Create'}
      </button>
      {error && <div className="text-sm" style={{ color: '#dc2626' }}>{error}</div>}
    </form>
  );
}
