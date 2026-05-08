import { describe, it, expect, beforeAll } from 'vitest';
import { signSessionCookie, verifySessionCookie } from './sessions.js';

describe('sessions', () => {
  beforeAll(() => { process.env.SESSION_SECRET = 'a'.repeat(48); });

  it('round-trips a signed claim', async () => {
    const jwt = await signSessionCookie({ sub: 'u1', email: 'a@b.c' });
    const claims = await verifySessionCookie(jwt);
    expect(claims.sub).toBe('u1');
    expect(claims.email).toBe('a@b.c');
  });

  it('rejects a tampered cookie', async () => {
    const jwt = await signSessionCookie({ sub: 'u1', email: 'a@b.c' });
    const tampered = jwt.slice(0, -2) + 'aa';
    await expect(verifySessionCookie(tampered)).rejects.toThrow();
  });
});
