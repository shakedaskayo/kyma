'use client';
import { useEffect, useState, Suspense } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import { api } from '@/lib/api';

function CallbackContent() {
  const params = useSearchParams();
  const router = useRouter();
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const token = params.get('token');
    if (!token) { setError('Missing token'); return; }
    api.exchangeMagicLink(token)
      .then(() => router.replace('/workspaces'))
      .catch((e) => setError(e.message));
  }, [params, router]);

  return (
    <div>
      {error
        ? <div className="text-sm" style={{ color: '#dc2626' }}>Sign-in failed: {error}</div>
        : <div className="text-sm">Signing you in…</div>}
    </div>
  );
}

export default function CallbackPage() {
  return (
    <main className="min-h-screen flex items-center justify-center px-4">
      <Suspense fallback={<div className="text-sm">Signing you in…</div>}>
        <CallbackContent />
      </Suspense>
    </main>
  );
}
