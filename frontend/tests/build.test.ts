import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { backendOrigin } from '../backend-origin.mjs';

test('build backend configuration rejects visitor-controlled or credential-bearing URLs', () => {
  assert.equal(backendOrigin(), 'https://backend.browser.teamofsilicons.com'); assert.equal(backendOrigin('http://127.0.0.1:8091/'), 'http://127.0.0.1:8091');
  for (const value of ['https://evil.example', 'https://a:b@backend.browser.teamofsilicons.com', 'http://backend.browser.teamofsilicons.com', 'https://backend.browser.teamofsilicons.com/api', 'https://backend.browser.teamofsilicons.com?backend=x', 'https://backend.browser.teamofsilicons.com#x']) assert.throws(()=>backendOrigin(value));
});
test('Vercel routes login and live handoffs to the static app and never proxies APIs', async () => {
  const config = JSON.parse(await readFile(new URL('../vercel.json', import.meta.url), 'utf8'));
  assert.equal(config.framework,'vite'); assert.equal(config.buildCommand,'npm run build'); assert.equal(config.outputDirectory,'dist');
  assert(config.rewrites.some((route:{source:string})=>route.source==='/auth/callback'));
  assert(config.rewrites.some((route:{source:string})=>route.source==='/sessions/:id/live'));
  assert(!config.rewrites.some((route:{source:string})=>route.source.includes('api')));
  const headers = Object.fromEntries(config.headers[0].headers.map((header:{key:string;value:string})=>[header.key,header.value]));
  assert.equal(headers['Referrer-Policy'],'no-referrer'); assert.equal(headers['Cache-Control'],'no-store'); assert(headers['Content-Security-Policy'].includes("font-src 'self'")); assert(!headers['Content-Security-Policy'].includes('unsafe-inline'));
});
test('static preview serves login/live deep links, immutable assets and never proxies API or dotfiles', async t => {
  const {mkdtemp, mkdir, copyFile, writeFile, rm} = await import('node:fs/promises');
  const {tmpdir} = await import('node:os'); const path = await import('node:path'); const {spawn} = await import('node:child_process');
  const dir = await mkdtemp(path.join(tmpdir(),'sb-solid-preview-'));
  await copyFile(new URL('../dev.mjs',import.meta.url),path.join(dir,'dev.mjs')); await copyFile(new URL('../vercel.json',import.meta.url),path.join(dir,'vercel.json'));
  await mkdir(path.join(dir,'dist/assets'),{recursive:true}); await writeFile(path.join(dir,'dist/index.html'),'<main>Browser</main>'); await writeFile(path.join(dir,'dist/assets/app-123.js'),'export {}');
  const child=spawn(process.execPath,['dev.mjs'],{cwd:dir,env:{...process.env,SB_FRONTEND_PORT:'0'},stdio:['ignore','pipe','pipe']});
  t.after(async()=>{if(child.exitCode===null){const closed = new Promise<void>(resolve=>child.once('close',()=>resolve()));child.kill();await closed;}await rm(dir,{recursive:true,force:true});});
  const origin = await new Promise<string>((resolve,reject)=>{const timeout=setTimeout(()=>reject(new Error('Preview did not start')),5000);child.once('error',error=>{clearTimeout(timeout);reject(error);});child.stdout.once('data',chunk=>{clearTimeout(timeout);const match=String(chunk).match(/http:\/\/127\.0\.0\.1:\d+/);match?resolve(match[0]):reject(new Error('Missing preview URL'));});});
  for(const route of ['/', '/auth/callback?nonce=n&slt=oac_test', '/sessions/s1/live']) { const response=await fetch(origin+route);assert.equal(response.status,200);assert.equal(await response.text(),'<main>Browser</main>');assert.equal(response.headers.get('cache-control'),'no-store');assert.equal(response.headers.get('referrer-policy'),'no-referrer'); }
  const asset=await fetch(origin+'/assets/app-123.js');assert.equal(asset.status,200);assert.equal(asset.headers.get('cache-control'),'public, max-age=31536000, immutable');
  for(const route of ['/api/v1/me','/.env','/assets/secret.txt','/assets/%2e%2e/.env']) assert.equal((await fetch(origin+route)).status,404);
});
