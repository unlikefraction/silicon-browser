-- Existing three-root consent cannot authorize upload-status reconciliation.
-- Require a new explicit consent before starting or delivering more recordings.
UPDATE recording_obo_grants SET needs_auth=1 WHERE enabled=1;
UPDATE recording_obo_authorizations SET status='cancelled',request_cipher='' WHERE status='pending';
