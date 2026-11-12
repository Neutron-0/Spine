-- 0014_maintenance_lease_recovery.sql
-- Add bounded lease_expires_at_nanos to _cluster_maintenance_barrier for crash/cancellation recovery.

ALTER TABLE _cluster_maintenance_barrier
    ADD COLUMN IF NOT EXISTS lease_expires_at_nanos BIGINT NOT NULL DEFAULT 0;
