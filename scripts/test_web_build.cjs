#!/usr/bin/env node
const { spawnSync } = require('node:child_process');
const path = require('node:path');
const result = spawnSync(process.execPath, ['--import', 'tsx', '--test', 'tests/build.test.ts'], { cwd: path.join(__dirname, '../frontend'), stdio: 'inherit' });
process.exit(result.status ?? 1);
