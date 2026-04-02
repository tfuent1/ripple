-- Migration 002: split submitted (outbound sent) from delivered (inbound processed).
-- Previously, mark_delivered was called for both outbound submission and inbound
-- receipt, overloading a single flag. Now:
--   submitted = 1  → bundle was POSTed to the rendezvous server
--   delivered  = 1 → bundle was received from the relay and processed locally
ALTER TABLE bundles ADD COLUMN submitted INTEGER NOT NULL DEFAULT 0;
