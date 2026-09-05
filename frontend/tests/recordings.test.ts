import test from 'node:test';
import assert from 'node:assert/strict';
import { recordingRecovery } from '../src/recordings';
import type { Recording } from '../src/types';

const pending: Recording = { session_id: 'session-1', owner_id: 'owner', session_name: 'Example', session_description: 'Example recording', status: 'pending', created_at: '2026-09-06T00:00:00Z', duration_seconds: 114, size_bytes: 0, delivery_error: 'recording_authorization_required' };

test('pending authorization failure offers the owner a reconnect action without requiring failed status', () => {
  const recovery = recordingRecovery(pending, 'owner');
  assert.equal(recovery.needsAuthorization, true);
  assert.equal(recovery.canReconnect, true);
  assert.equal(recovery.canRetry, false);
  assert.match(recovery.message, /resume saving/);
  assert(!recovery.message.includes('recording_authorization_required'));
});

test('shared recording viewers cannot renew the owner grant or retry delivery', () => {
  for (const identity of ['viewer', undefined]) {
    const recovery = recordingRecovery({ ...pending, status: 'failed' }, identity);
    assert.equal(recovery.canReconnect, false);
    assert.equal(recovery.canRetry, false);
    assert.match(recovery.message, /session owner/);
  }
});

test('ordinary failures offer owner retry and completed deliveries remove recovery actions', () => {
  assert.equal(recordingRecovery({ ...pending, status: 'failed', delivery_error: 'delivery_timeout' }, 'owner').canRetry, true);
  const recovery = recordingRecovery({ ...pending, status: 'available', delivery_error: undefined }, 'owner');
  assert.equal(recovery.canReconnect, false);
  assert.equal(recovery.canRetry, false);
  assert.equal(recovery.message, '');
});

test('a missing native recording cannot offer a retry that the server will reject', () => {
  const recovery = recordingRecovery({ ...pending, status: 'failed', delivery_error: 'native_recording_unavailable' }, 'owner');
  assert.equal(recovery.canRetry, false);
  assert.equal(recovery.canReconnect, false);
  assert.match(recovery.message, /did not provide a recording/);
});
