import type { NextConfig } from 'next';
const apiUrl = process.env.API_BASE_URL ?? process.env.NEXT_PUBLIC_API_BASE_URL ?? 'http://localhost:3001';
const config: NextConfig = {
  output: 'standalone',
  transpilePackages: ['@kyma/shared'],
  experimental: { externalDir: true },
  async rewrites() {
    return [{ source: '/api/:path*', destination: `${apiUrl}/api/:path*` }];
  },
};
export default config;
