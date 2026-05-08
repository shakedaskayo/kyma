// src/app/(dashboard)/layout.tsx
import { redirect } from 'next/navigation';
import { getCurrentUser } from '@/lib/auth-server';
import Link from 'next/link';

export default async function DashboardLayout({ children }: { children: React.ReactNode }) {
  const user = await getCurrentUser();
  if (!user) redirect('/login');
  return (
    <div className="min-h-screen">
      <header
        className="px-6 h-14 flex items-center justify-between border-b"
        style={{ borderColor: 'var(--kyma-rule-soft)' }}
      >
        <Link href="/workspaces" className="font-mono font-semibold">kyma cloud</Link>
        <div className="text-sm" style={{ color: 'var(--kyma-muted)' }}>{user.email}</div>
      </header>
      <main className="px-6 py-8 max-w-5xl mx-auto">{children}</main>
    </div>
  );
}
