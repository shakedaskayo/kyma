import { listWorkspaces } from '@/lib/db';

export default async function AdminWorkspacesPage() {
  const rows = await listWorkspaces();
  return (
    <div>
      <h1 className="text-xl mb-4">Workspaces ({rows.length})</h1>
      <table className="w-full text-sm">
        <thead className="text-left" style={{ color: 'var(--kyma-muted)' }}>
          <tr><th>Slug</th><th>Name</th><th>Owner</th><th>Plan</th><th>Kind</th><th>Created</th></tr>
        </thead>
        <tbody>
          {rows.map((w) => (
            <tr key={w.id} className="border-t" style={{ borderColor: 'var(--kyma-rule-soft)' }}>
              <td className="py-2 font-mono">{w.slug}</td><td>{w.name}</td>
              <td>{w.owner_email}</td><td>{w.plan}</td><td>{w.kind}</td>
              <td style={{ color: 'var(--kyma-muted)' }}>{new Date(w.created_at).toISOString().slice(0, 10)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="text-xs mt-6" style={{ color: 'var(--kyma-muted)' }}>
        &quot;Promote to dedicated&quot; lands in Slice 3.
      </p>
    </div>
  );
}
