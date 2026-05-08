import { cookies } from 'next/headers';
import { redirect } from 'next/navigation';
import Link from 'next/link';
import { ADMIN_COOKIE, verifyAdmin } from '@/lib/admin-session';

export default async function AdminDashLayout({ children }: { children: React.ReactNode }) {
  const c = await cookies();
  const cookie = c.get(ADMIN_COOKIE)?.value;
  const ok = cookie ? await verifyAdmin(cookie) : null;
  if (!ok) redirect('/');
  return (
    <div className="min-h-screen">
      <header className="px-6 h-14 flex items-center justify-between border-b" style={{ borderColor: 'var(--kyma-rule-soft)' }}>
        <Link href="/workspaces" className="font-mono">kyma admin · {ok.ghLogin}</Link>
      </header>
      <main className="px-6 py-8 max-w-6xl mx-auto">{children}</main>
    </div>
  );
}
