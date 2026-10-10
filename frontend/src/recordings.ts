import type { Recording } from './types';

export function recordingStatusMessage(item: Recording): string {
  if (item.status === 'available' || item.status === 'trashed') return '';
  const messages: Record<string, string> = {
    recording_source_unavailable: 'The browser service has not made a recording available. Refresh to check its status.',
    native_recording_unavailable: 'The browser service did not provide a recording for this session.',
  };
  if (item.delivery_error) return messages[item.delivery_error] || 'This recording is currently unavailable.';
  return item.status === 'pending' ? 'This recording is not available yet. Refresh to check its status.' : '';
}
