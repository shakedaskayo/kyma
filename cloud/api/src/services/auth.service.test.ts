import { describe, it, expect, beforeAll } from 'vitest';
import { buildGithubAuthorizeUrl, newOauthState } from './auth.service.js';

beforeAll(() => { process.env.GITHUB_CLIENT_ID = 'client_xyz'; process.env.SESSION_SECRET = 'a'.repeat(48); });

describe('auth.service', () => {
  it('newOauthState returns 32 hex chars', () => {
    const s = newOauthState();
    expect(s.length).toBe(32);
    expect(/^[0-9a-f]+$/.test(s)).toBe(true);
  });
  it('buildGithubAuthorizeUrl encodes redirect + state', () => {
    const url = buildGithubAuthorizeUrl('abc', 'http://localhost/cb');
    expect(url).toContain('client_id=client_xyz');
    expect(url).toContain('state=abc');
    expect(url).toContain('redirect_uri=http%3A%2F%2Flocalhost%2Fcb');
  });
});
