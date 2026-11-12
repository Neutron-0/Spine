-- 0013_cluster_barrier_fencing_generation.sql
-- Ensure _cluster_maintenance_barrier carries the fencing_generation column required by ConsistencyBarrier.

ALTER TABLE _cluster_maintenance_barrier
    ADD COLUMN IF NOT EXISTS fencing_generation BIGINT NOT NULL DEFAULT 0;
