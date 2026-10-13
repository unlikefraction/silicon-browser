import test from 'node:test';
import assert from 'node:assert/strict';
import { recordingStatusMessage } from '../src/lib/recordings';
import type { Recording } from '../src/lib/types';

const pending: Recording = { session_id: 'session-1', owner_id: 'owner', session_name: 'Example', session_description: 'Example recording', status: 'pending', created_at: '2026-09-06T00:00:00Z', duration_seconds: 114, size_bytes: 0, delivery_error: 'recording_authorization_required' };

test('retired delivery failures do not suggest removed authorization or retry flows', () => {
  const message = recordingStatusMessage(pending);
  assert.equal(message, 'This recording is currently unavailable.');
});

test('provider source availability can be checked without promising automatic delivery', () => {
  const message = recordingStatusMessage({ ...pending, delivery_error: 'recording_source_unavailable' });
  assert.match(message, /Refresh to check its status/);
  assert.doesNotMatch(message, /delivery|automatically/);
});

test('available provider replays do not display stale legacy delivery errors', () => {
  assert.equal(recordingStatusMessage({ ...pending, status: 'available' }), '');
});

test('missing provider recordings retain the specific status detail', () => {
  assert.match(recordingStatusMessage({ ...pending, status: 'failed', delivery_error: 'native_recording_unavailable' }), /did not provide a recording/);
});
