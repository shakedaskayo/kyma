import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { freshDb } from '../test-setup.js';
import { closeDb } from '../db/client.js';
import { issueMagicLink, exchangeMagicLink } from './auth.service.js';

describe('magic-link round-trip', () => {
  beforeAll(async () => {
    process.env.SESSION_SECRET = 'a'.repeat(48);
    await freshDb();
  });
  afterAll(async () => { await closeDb(); });

  it('issues and exchanges, creating user on first exchange', async () => {
    const { link } = await issueMagicLink('first@kyma.dev');
    const token = link.split('token=')[1];
    const { user } = await exchangeMagicLink(token);
    expect(user.email).toBe('first@kyma.dev');
  });

  it('rejects re-exchange of consumed token', async () => {
    const { link } = await issueMagicLink('again@kyma.dev');
    const token = link.split('token=')[1];
    await exchangeMagicLink(token);
    await expect(exchangeMagicLink(token)).rejects.toThrow();
  });
});
