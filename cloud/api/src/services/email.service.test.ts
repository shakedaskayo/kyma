import { describe, it, expect, beforeAll, vi } from 'vitest';
import { sendMagicLinkEmail } from './email.service.js';

beforeAll(() => { delete process.env.RESEND_API_KEY; });

describe('email.service', () => {
  it('logs the link in dev when no Resend key', async () => {
    const log = vi.spyOn(console, 'log').mockImplementation(() => {});
    await sendMagicLinkEmail('a@b.c', 'http://example/link');
    expect(log).toHaveBeenCalledWith(expect.stringContaining('http://example/link'));
    log.mockRestore();
  });
});
