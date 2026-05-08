import { createHash, randomBytes } from 'node:crypto';

const TOKEN_PREFIX = 'kyma_';

export function generateMcpToken(): { plain: string; hash: Buffer; prefix: string } {
  const raw = randomBytes(32).toString('hex');
  const plain = `${TOKEN_PREFIX}${raw}`;
  const prefix = `${TOKEN_PREFIX}${raw.slice(0, 8)}`;
  const hash = hashToken(plain);
  return { plain, hash, prefix };
}

/** Returns the raw 32-byte SHA-256 — matches the engine's DbAuthBackend hashing. */
export function hashToken(plain: string): Buffer {
  return createHash('sha256').update(plain).digest();
}

export function randomBytesHex(n: number): string {
  return randomBytes(n).toString('hex');
}
