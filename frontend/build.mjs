// Compatibility entrypoint. Solid/Vite owns the frontend build.
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
const result = spawnSync('npm', ['run', 'build'], { cwd: fileURLToPath(new URL('.', import.meta.url)), stdio: 'inherit' });
process.exit(result.status ?? 1);
