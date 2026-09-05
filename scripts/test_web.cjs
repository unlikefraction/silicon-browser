#!/usr/bin/env node
// Compatibility entrypoint for the Solid frontend's behavior contracts.
const { spawnSync } = require('node:child_process');
const path = require('node:path');
const result = spawnSync(process.execPath, ['--import', 'tsx', '--test', 'tests/api.test.ts', 'tests/auth.test.ts'], { cwd: path.join(__dirname, '../frontend'), stdio: 'inherit' });
process.exit(result.status ?? 1);
