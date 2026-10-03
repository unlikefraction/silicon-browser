import type { BrowserApi } from './api';
export interface RecordingConsent { authorization_id:string;consent_url:string|null;state:string;status:'pending'|'completed';expires_at:string }
function context(api:BrowserApi){const session=api.currentSession();if(!session)throw new Error('Sign in to approve recording access.');return JSON.stringify([api.origin,session.identity.id,session.org.id]);}
export function acceptRecordingConsent(value:RecordingConsent):RecordingConsent{
 if(!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value?.authorization_id||'')||! /^[0-9a-f]{64}$/.test(value.state||'')||!['pending','completed'].includes(value.status)||!Number.isFinite(Date.parse(value.expires_at)))throw new Error('IAM returned an invalid recording approval. Start a new request.');
 if(value.status==='pending'){
  if(!value.consent_url)throw new Error('IAM did not return an approval page.');const url=new URL(value.consent_url);
  if(!(url.protocol==='https:'||(url.protocol==='http:'&&['localhost','127.0.0.1','[::1]'].includes(url.hostname)))||url.username||url.password||url.hash)throw new Error('The recording approval link is invalid.');
 }
 return value;
}
/** All pending approval state is memory-only and bound to the exact API, account and organization. */
export class RecordingConsentFlow{
 private completionCode:string|undefined;private generation=0;private retryKey:string|undefined;private pending:RecordingConsent|undefined;private owner:{api:BrowserApi;context:string}|undefined;
 reset(){this.completionCode=undefined;this.generation++;this.retryKey=undefined;this.pending=undefined;this.owner=undefined;}
 private bind(api:BrowserApi){const key=context(api);if(this.owner&&(this.owner.api!==api||this.owner.context!==key))this.reset();this.owner={api,context:key};return{api,key,generation:this.generation};}
 private ensure(ticket:ReturnType<RecordingConsentFlow['bind']>,current:BrowserApi){if(ticket.generation!==this.generation||current!==ticket.api||context(current)!==ticket.key)throw new Error('Account or organization changed. Start recording approval in the current workspace.');}
 async start(current:()=>BrowserApi,popup=false):Promise<RecordingConsent>{const ticket=this.bind(current());this.retryKey??=crypto.randomUUID();const result=await ticket.api.call<RecordingConsent>('/auth/delivery/authorizations','POST',popup?{popup:true}:{},this.retryKey);this.ensure(ticket,current());this.pending=acceptRecordingConsent(result);return this.pending;}
 hasCompletion(){return this.completionCode!==undefined;}
 async retry(current:()=>BrowserApi):Promise<void>{const code=this.completionCode;if(!code)throw new Error("No approval is waiting to be saved.");await this.complete(current,code);}
 async complete(current:()=>BrowserApi,raw:string):Promise<void>{
  const ticket=this.bind(current()),pending=this.pending;if(!pending)throw new Error('Start recording approval in the current workspace first.');const code=raw.trim();
  if(!/^obc_[\x21-\x7e]{1,16380}$/.test(code))throw new Error('Paste the approval code returned by IAM.');
  this.completionCode=code;
  // Backend retry identity binds this exact code; corrected codes are distinct attempts.
  await ticket.api.call(`/auth/delivery/authorizations/${encodeURIComponent(pending.authorization_id)}/complete`,'POST',{code,state:pending.state});this.ensure(ticket,current());this.reset();
 }
}
