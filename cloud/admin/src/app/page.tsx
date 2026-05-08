import Link from 'next/link';

export default function AdminLogin() {
  return (
    <main className="min-h-screen flex items-center justify-center px-4">
      <div className="space-y-3 text-center">
        <h1 className="text-2xl font-mono">kyma admin</h1>
        <Link
          href="/api/auth/github/start"
          className="inline-block h-10 px-4 rounded text-white"
          style={{ background: 'var(--kyma-accent)' }}
        >
          Sign in with GitHub
        </Link>
      </div>
    </main>
  );
}
