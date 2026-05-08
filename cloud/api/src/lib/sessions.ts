import * as jose from 'jose';
import { getEnv } from '../env.js';

export interface SessionClaims {
  sub: string;
  email: string;
}

const ISS = 'kyma-cloud';
const COOKIE_NAME = 'kyma_session';
const TTL = '30d';

function secret(): Uint8Array {
  return new TextEncoder().encode(getEnv().SESSION_SECRET);
}

export async function signSessionCookie(claims: SessionClaims): Promise<string> {
  return new jose.SignJWT({ ...claims })
    .setProtectedHeader({ alg: 'HS256' })
    .setIssuedAt()
    .setExpirationTime(TTL)
    .setIssuer(ISS)
    .sign(secret());
}

export async function verifySessionCookie(jwt: string): Promise<SessionClaims> {
  const { payload } = await jose.jwtVerify(jwt, secret(), { issuer: ISS });
  return { sub: payload.sub as string, email: payload.email as string };
}

export const SESSION_COOKIE_NAME = COOKIE_NAME;
