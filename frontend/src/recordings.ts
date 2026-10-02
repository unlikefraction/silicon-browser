import type { Recording } from './types';

export function recordingRecovery(item: Recording, identityId?: string) {
  const owner = Boolean(identityId && item.owner_id === identityId);
  const needsAuthorization = ['recording_authorization_required', 'recording_destination_changed'].includes(item.delivery_error || '');
  const messages: Record<string, string> = {
    recording_authorization_required: owner
      ? 'Recording access needs to be renewed. Reconnect to resume saving this recording to your Briefcase.'
      : 'The session owner needs to reconnect recording access before this recording can be saved.',
    recording_destination_changed: owner
      ? 'Restore the original Briefcase account and organization in recording approval to resume this delivery. A started upload cannot be moved to another destination.'
      : 'The session owner must restore the original Briefcase account and organization before this delivery can resume.',
    recording_authorization_refreshing: 'Recording access is being renewed. Delivery will resume automatically.',
    recording_source_unavailable: 'Your recording is still being prepared. Delivery will retry automatically.',
    recording_proof_unavailable: 'Recording delivery is temporarily unavailable. Browser will retry automatically.',
    briefcase_upload_unconfirmed: 'Briefcase has not confirmed delivery yet. Browser will check again automatically.',
    native_recording_unavailable: 'The browser service did not provide a recording for this session.',
    recording_size_limit: 'This recording exceeds the current delivery size limit.',
  };
  return {
    message: item.delivery_error ? messages[item.delivery_error] || 'This recording could not be delivered. Refresh its status or retry delivery if available.' : '',
    needsAuthorization,
    canReconnect: owner && needsAuthorization,
    canRetry: owner && item.status === 'failed' && [
      'recording_size_limit', 'recording_source_unavailable', 'recording_proof_unavailable',
      'recording_proof_expired', 'briefcase_upload_unconfirmed', 'delivery_timeout', 'delivery_attempts_exhausted',
    ].includes(item.delivery_error || ''),
  };
}
