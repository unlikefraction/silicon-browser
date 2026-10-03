import test from 'node:test';
import assert from 'node:assert/strict';
import { BrowserApi } from '../src/api';
import { RecordingConsentFlow, acceptRecordingConsent } from '../src/recording-consent';
import type { AuthSession } from '../src/types';
const session=(org='tos'):AuthSession=>({access_token:'oat_initial',refresh_token:'ort_original',expires_at:new Date(Date.now()+900000).toISOString(),identity:{id:'c:person',name:'Person',kind:'carbon'},org:{id:org,name:org}});
const pending={authorization_id:'ed5aa54c-687c-4397-bbd1-29f10db65c10',consent_url:'https://auth.iam.example/obo/consent',state:'a'.repeat(64),status:'pending' as const,expires_at:'2099-01-01T00:00:00Z'};
function api(handler:(url:string,options:RequestInit)=>Promise<Response>){const client=new BrowserApi('https://backend.browser.example',((url,options)=>handler(String(url),options||{}))as typeof fetch);client.setSession(session());return client;}
const response=(data:unknown)=>new Response(JSON.stringify({data}),{status:200});
test('lost start response reuses one key; bad code retains login and accepts a corrected code',async()=>{
 const keys:string[]=[],codes:string[]=[];let calls=0;const client=api(async(url,options)=>{
  assert(!(url.includes('/sessions')||url.includes('/auth/refresh')));
  if(url.endsWith('/complete')){const body=JSON.parse(String(options.body));codes.push(body.code);assert.equal(body.state,pending.state);if(codes.length===1)return new Response(JSON.stringify({error:{message:'Invalid approval code'}}),{status:400});return response({...pending,status:'completed'});}
  keys.push(new Headers(options.headers).get('idempotency-key')||'');if(calls++===0)throw new Error('response lost');return response(pending);
 });
 const flow=new RecordingConsentFlow();await assert.rejects(flow.start(()=>client),/Connection interrupted/);await flow.start(()=>client);assert.equal(keys[0],keys[1]);assert(keys[0]);
 await assert.rejects(flow.complete(()=>client,'obc_wrong'),/Invalid approval/);assert.equal(client.currentSession()?.access_token,'oat_initial');await flow.complete(()=>client,'obc_correct');assert.deepEqual(codes,['obc_wrong','obc_correct']);
});
test('late approval from a previous API or testing plane cannot enter the new workspace',async()=>{
 let resolve!:(r:Response)=>void;const old=api(async()=>new Promise(r=>{resolve=r}));const other=api(async()=>response(pending));let current=old;
 const flow=new RecordingConsentFlow();const started=flow.start(()=>current);current=other;resolve(response(pending));await assert.rejects(started,/changed/);await assert.rejects(flow.complete(()=>current,'obc_correct'),/Start recording approval/);
});
test('account/org changes and cancellation invalidate pending completion',async()=>{
 let writes=0;const client=api(async(url)=>{if(url.endsWith('/complete'))writes++;return response(pending)});const flow=new RecordingConsentFlow();await flow.start(()=>client);client.setSession(session('other'));
 await assert.rejects(flow.complete(()=>client,'obc_correct'),/Start recording approval/);assert.equal(writes,0);await flow.start(()=>client);flow.reset();await assert.rejects(flow.complete(()=>client,'obc_correct'),/Start recording approval/);assert.equal(writes,0);
});
test('consent URL refuses credentials, fragments and remote HTTP',()=>{
 for(const consent_url of ['javascript:alert(1)','http://auth.example/consent','https://user:secret@auth.example/consent','https://auth.example/consent#secret'])assert.throws(()=>acceptRecordingConsent({...pending,consent_url}));
 assert.equal(acceptRecordingConsent({...pending,consent_url:'http://127.0.0.1:4310/obo/consent'}).status,'pending');
});

test('an uncertain completion retains the exact code for retry and clears it on success',async()=>{
 const bodies:string[]=[];const client=api(async(url,options)=>{if(!url.endsWith('/complete'))return response(pending);bodies.push(String(options.body));if(bodies.length===1)throw new Error('response lost');return response({...pending,status:'completed'});});
 const flow=new RecordingConsentFlow();await flow.start(()=>client,true);await assert.rejects(flow.complete(()=>client,'obc_approved'),/Connection interrupted/);assert.equal(flow.hasCompletion(),true);await flow.retry(()=>client);assert.equal(bodies[0],bodies[1]);assert.equal(flow.hasCompletion(),false);await assert.rejects(flow.retry(()=>client),/No approval/);
});
test('saved approval retry cannot cross accounts or survive cancellation',async()=>{
 let writes=0;const client=api(async(url)=>{if(!url.endsWith('/complete'))return response(pending);writes++;throw new Error('response lost');});const flow=new RecordingConsentFlow();await flow.start(()=>client);await assert.rejects(flow.complete(()=>client,'obc_approved'));client.setSession(session('other'));await assert.rejects(flow.retry(()=>client),/Start recording approval/);assert.equal(writes,1);assert.equal(flow.hasCompletion(),false);await flow.start(()=>client);await assert.rejects(flow.complete(()=>client,'obc_approved'));flow.reset();await assert.rejects(flow.retry(()=>client),/No approval/);assert.equal(writes,2);
});
