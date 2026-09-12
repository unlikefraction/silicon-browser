export interface Identity { id: string; name: string; kind: 'carbon' | 'silicon' }
export interface Organization { id: string; name: string }
export interface AuthSession { access_token: string; refresh_token: string; expires_at: string; identity: Identity; org: { id: string; name: string }; services?: string[] }
export interface Location { code: string; name: string }
export interface Profile { id: string; name: string; fingerprint: string; location: Location; access: string[]; owner_id: string; sessions_run: number; status: string }
export interface Usage { session_id?: string; sessions?: number; browser_seconds: number; proxy_bytes_in: number; proxy_bytes_out: number; proxy_bytes_unclassified?: number; cost?: { total?: { micros: number; currency: string } } }
export interface UsageLimits { concurrent_browser_limit: number; rate_limit?: number | null; checked_at: string }
export interface Session { id: string; profile_id?: string; name: string; description: string; status: string; initiator_id: string; participant_ids?: string[]; started_at: string; expires_at: string; usage: Usage }
export interface Recording { session_id: string; owner_id: string; session_name: string; session_description: string; status: string; created_at: string; duration_seconds: number; size_bytes: number; briefcase_link?: string; command_log_link?: string; delivery_error?: string }
export interface Delivery { configured: boolean; enabled: boolean; state: string; actor_id: string }
export interface SessionLog { command: string; created_at?: string }
export interface PendingLive { id: string; grant: string }
