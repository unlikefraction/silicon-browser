-- A previously attempted command artifact is bound to its exact JSONL bytes.
-- Keep its historical actor spelling only for regenerating those frozen bytes;
-- commands.actor_id remains the canonical identity used for access and AAD.
ALTER TABLE commands ADD COLUMN delivery_actor_id TEXT;
