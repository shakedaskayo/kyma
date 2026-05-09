import { NextRequest } from 'next/server';

export const dynamic = 'force-dynamic';
export const runtime = 'nodejs';

function targetUrl(req: NextRequest): string {
  const base = process.env.API_BASE_URL ?? process.env.NEXT_PUBLIC_API_BASE_URL ?? '';
  if (!base) throw new Error('API_BASE_URL not configured');
  const url = new URL(req.url);
  return `${base.replace(/\/$/, '')}${url.pathname}${url.search}`;
}

async function proxy(req: NextRequest): Promise<Response> {
  const target = targetUrl(req);
  const headers = new Headers(req.headers);
  headers.delete('host');
  headers.set('x-forwarded-host', req.headers.get('host') ?? '');
  headers.set('x-forwarded-proto', 'https');

  const init: RequestInit = {
    method: req.method,
    headers,
    redirect: 'manual',
  };
  if (req.method !== 'GET' && req.method !== 'HEAD') {
    init.body = await req.arrayBuffer();
  }
  const upstream = await fetch(target, init);
  const respHeaders = new Headers(upstream.headers);
  return new Response(upstream.body, {
    status: upstream.status,
    statusText: upstream.statusText,
    headers: respHeaders,
  });
}

export const GET = proxy;
export const POST = proxy;
export const PUT = proxy;
export const PATCH = proxy;
export const DELETE = proxy;
export const HEAD = proxy;
export const OPTIONS = proxy;
