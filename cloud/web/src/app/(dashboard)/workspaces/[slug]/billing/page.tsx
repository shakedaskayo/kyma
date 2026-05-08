'use client';
import { useEffect, useState } from 'react';
import { useParams } from 'next/navigation';
import { api } from '@/lib/api';
import { PLANS, type PlanId } from '@kyma/shared/plans';
import { UsageChart } from '@/components/usage-chart';

type Subscription = {
  plan: string;
  planActive: boolean;
  trialEndsAt: string | null;
  currentPeriodEnd: string | null;
  dunningState: string | null;
  stripeCustomerId?: string | null;
};

type DayRow = { day: string; mcpCalls: number; ingestBytes: number };

export default function BillingPage() {
  const params = useParams<{ slug: string }>();
  const slug = params.slug;

  const [sub, setSub] = useState<Subscription | null>(null);
  const [usage, setUsage] = useState<DayRow[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([
      api.billing.subscription(slug),
      fetch(`${process.env.NEXT_PUBLIC_CLOUD_BASE_URL ?? ''}/api/usage/${slug}/daily`, {
        credentials: 'include',
      }).then((r) => r.json() as Promise<{ usage: DayRow[] }>),
    ])
      .then(([s, { usage: rows }]) => {
        setSub(s as Subscription);
        setUsage(rows);
      })
      .catch((e) => setError(e.message));
  }, [slug]);

  async function checkout(plan: 'pro' | 'team') {
    setBusy(plan);
    try {
      const { url } = await api.billing.checkout(slug, plan);
      window.location.href = url;
    } catch (e: any) {
      setError(e.message);
      setBusy(null);
    }
  }

  async function portal() {
    setBusy('portal');
    try {
      const { url } = await api.billing.portal(slug);
      window.location.href = url;
    } catch (e: any) {
      setError(e.message);
      setBusy(null);
    }
  }

  if (error) return <div style={{ color: '#dc2626' }}>{error}</div>;
  if (!sub) return <div>Loading…</div>;

  const planIds: PlanId[] = ['free', 'pro', 'team'];

  return (
    <div className="space-y-8">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="space-y-1">
          <h1 className="text-xl">Billing</h1>
          <p className="text-sm" style={{ color: 'var(--kyma-muted)' }}>
            Current plan:{' '}
            <span className="font-medium capitalize">{sub.plan}</span>
            {!sub.planActive && (
              <span className="ml-2 text-xs px-2 py-0.5 rounded"
                    style={{ background: '#fef3c7', color: '#92400e' }}>
                inactive
              </span>
            )}
          </p>
        </div>
        {sub.stripeCustomerId && (
          <button
            onClick={portal}
            disabled={busy === 'portal'}
            className="h-9 px-4 rounded border text-sm"
            style={{ borderColor: 'var(--kyma-rule-soft)' }}
          >
            {busy === 'portal' ? 'Redirecting…' : 'Manage billing'}
          </button>
        )}
      </div>

      {/* Dunning warning */}
      {sub.dunningState && (
        <div className="rounded p-4 text-sm"
             style={{ background: '#fef3c7', color: '#92400e' }}>
          Payment issue detected ({sub.dunningState}). Please update your payment method to avoid
          service interruption.
        </div>
      )}

      {/* Plan grid */}
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
        {planIds.map((id) => {
          const p = PLANS[id];
          const isCurrent = sub.plan === id;
          return (
            <div
              key={id}
              className="rounded border p-5 space-y-4 flex flex-col"
              style={{
                borderColor: isCurrent ? 'var(--kyma-accent)' : 'var(--kyma-rule-soft)',
                background: isCurrent ? 'var(--kyma-bg-soft)' : undefined,
              }}
            >
              <div className="space-y-1">
                <div className="flex items-center justify-between">
                  <span className="font-medium capitalize">{id}</span>
                  {isCurrent && (
                    <span className="text-xs px-2 py-0.5 rounded text-white"
                          style={{ background: 'var(--kyma-accent)' }}>
                      current
                    </span>
                  )}
                </div>
                <p className="text-2xl font-bold">
                  {p.pricePerMonth === 0 ? 'Free' : `$${p.pricePerMonth}/mo`}
                </p>
              </div>

              <ul className="space-y-1 text-sm flex-1" style={{ color: 'var(--kyma-fg-soft)' }}>
                {p.features.map((f) => (
                  <li key={f} className="flex gap-2">
                    <span style={{ color: 'var(--kyma-accent)' }}>✓</span>
                    {f}
                  </li>
                ))}
              </ul>

              {id !== 'free' && !isCurrent && (
                <button
                  onClick={() => checkout(id as 'pro' | 'team')}
                  disabled={busy === id}
                  className="h-9 px-4 rounded text-white text-sm w-full"
                  style={{ background: 'var(--kyma-accent)' }}
                >
                  {busy === id ? 'Redirecting…' : `Upgrade to ${id}`}
                </button>
              )}
            </div>
          );
        })}
      </div>

      {/* Usage charts */}
      <section className="space-y-4">
        <h2 className="text-base">Usage</h2>
        {usage.length === 0 ? (
          <p className="text-sm" style={{ color: 'var(--kyma-muted)' }}>
            No usage data for the last 30 days.
          </p>
        ) : (
          <UsageChart rows={usage} />
        )}
      </section>
    </div>
  );
}
