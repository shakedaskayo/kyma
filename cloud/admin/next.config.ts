import type { NextConfig } from 'next';
const config: NextConfig = {
  output: 'standalone',
  transpilePackages: ['@kyma/shared'],
  experimental: { externalDir: true },
};
export default config;
