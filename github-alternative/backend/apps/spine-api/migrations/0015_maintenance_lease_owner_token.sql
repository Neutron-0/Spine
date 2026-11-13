-- 0015_maintenance_lease_owner_token.sql
-- Add unique owner_token to _cluster_maintenance_barrier for CAS lease ownership and expiry verification.

ALTER TABLE _cluster_maintenance_barrier
    ADD COLUMN IF NOT EXISTS owner_token VARCHAR(64) NOT NULL DEFAULT '';
