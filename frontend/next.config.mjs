// Static export only: no server code, rewrites, headers, or redirects. Hosting (nginx, mirrored by vercel.json) owns routes and headers.
import { backendOrigin } from './backend-origin.mjs';

/** @type {import('next').NextConfig} */
const nextConfig = {
  output: 'export',
  trailingSlash: false,
  reactStrictMode: true,
  productionBrowserSourceMaps: false,
  poweredByHeader: false,
  images: { unoptimized: true },
  // `next dev` would otherwise write AGENTS.md and CLAUDE.md into this folder.
  agentRules: false,
  env: { SB_BACKEND_ORIGIN: backendOrigin(process.env.SB_PUBLIC_BACKEND_URL) },
};

export default nextConfig;
