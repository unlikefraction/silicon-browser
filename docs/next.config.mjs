import path from "node:path";
import { fileURLToPath } from "node:url";

const docsDir = path.dirname(fileURLToPath(import.meta.url));

/** @type {import('next').NextConfig} */
const nextConfig = {
  turbopack: {
    root: path.join(docsDir, ".."),
  },
};

export default nextConfig;
