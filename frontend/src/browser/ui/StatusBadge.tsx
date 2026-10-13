import { Badge, type BadgeSize } from '@/components/silicon-ui/badge/badge';
import { statusLabel } from '@/lib/format';

/** A session, profile, or recording status: `active` and `complete` read as success, everything else neutral; `_` shows as a space. */
export function StatusBadge({ status, size = 'sm' }: { status: string; size?: BadgeSize }) {
  return <Badge tone={status === 'active' || status === 'complete' ? 'success' : 'neutral'} size={size}>{statusLabel(status)}</Badge>;
}
