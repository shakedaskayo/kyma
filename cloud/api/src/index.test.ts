import { describe, it, expect } from 'vitest';
import { buildApp } from './index.js';

describe('app', () => {
  it('GET /health returns ok', async () => {
    const res = await buildApp().request('/health');
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ status: 'ok', service: 'kyma-cloud-api' });
  });
  it('GET /unknown returns 404 envelope', async () => {
    const res = await buildApp().request('/unknown');
    expect(res.status).toBe(404);
    const body = await res.json() as any;
    expect(body.error.code).toBe('NOT_FOUND');
  });
});
