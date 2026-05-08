import { Resend } from 'resend';
import { getEnv } from '../env.js';

let _resend: Resend | null = null;
function client() {
  if (_resend) return _resend;
  const key = getEnv().RESEND_API_KEY;
  if (!key) throw new Error('RESEND_API_KEY not set');
  _resend = new Resend(key);
  return _resend;
}

export async function sendMagicLinkEmail(to: string, link: string): Promise<void> {
  const env = getEnv();
  if (!env.RESEND_API_KEY) {
    console.log(`[email] DEV: magic link for ${to}: ${link}`);
    return;
  }
  await client().emails.send({
    from: env.RESEND_FROM_EMAIL,
    to,
    subject: 'Sign in to kyma cloud',
    html: `
      <div style="font-family: 'IBM Plex Sans', system-ui, sans-serif; max-width: 480px; margin: 0 auto; padding: 32px;">
        <h1 style="font-family: 'JetBrains Mono', ui-monospace, monospace; font-size: 22px; margin: 0 0 16px;">kyma cloud</h1>
        <p>Click the link below to sign in. It expires in 15 minutes.</p>
        <p style="margin: 24px 0;">
          <a href="${link}"
             style="display: inline-block; background: #2d6f1f; color: white; padding: 12px 24px;
                    border-radius: 6px; text-decoration: none; font-weight: 600;">
            Sign in to kyma cloud
          </a>
        </p>
        <p style="color: #767880; font-size: 13px;">
          If you didn't request this, you can safely ignore this email.
        </p>
      </div>
    `,
  });
}
