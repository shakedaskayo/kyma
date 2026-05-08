import { describe, it, expect } from 'vitest';
import { createHash } from 'node:crypto';
import { generateMcpToken, hashToken } from './tokens.js';

describe('tokens', () => {
  it('mints a 69-char token with kyma_ prefix and a 32-byte hash', () => {
    const t = generateMcpToken();
    expect(t.plain.length).toBe(69);
    expect(t.plain.startsWith('kyma_')).toBe(true);
    expect(t.hash.length).toBe(32);
    expect(t.prefix.length).toBe(13);
  });

  it('hashToken matches plain crypto.createHash output (engine compat)', () => {
    const expected = createHash('sha256').update('kyma_x').digest();
    expect(hashToken('kyma_x').equals(expected)).toBe(true);
  });
});
