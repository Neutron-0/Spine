#!/usr/bin/env bash
set -euo pipefail

# ==============================================================================
# Spine Forge - Secure PostgreSQL Setup (WSL / Local Linux)
# Least Privilege & Loopback Network Binding Enforcement
# ==============================================================================

POSTGRES_VERSION="18"
CONF_DIR="/etc/postgresql/${POSTGRES_VERSION}/main"
DB_NAME="${SPINE_POSTGRES_DB:-spine_forge}"
APP_USER="${SPINE_POSTGRES_USER:-spine_app}"
APP_PASSWORD="${SPINE_POSTGRES_PASSWORD:-spine_secure_dev_password}"

echo "[INFO] Configuring PostgreSQL network binding to loopback only..."
# Restrict listen_addresses to loopback and local WSL interface
if [ -f "${CONF_DIR}/postgresql.conf" ]; then
    sed -i "s/^#\?listen_addresses\s*=.*/listen_addresses = 'localhost,127.0.0.1'/" "${CONF_DIR}/postgresql.conf"
fi

# Configure host-based authentication (pg_hba.conf) with least privilege
# Disallow 0.0.0.0/0 broad network exposure. Only permit loopback and local host subnet.
if [ -f "${CONF_DIR}/pg_hba.conf" ]; then
    # Remove broad 0.0.0.0/0 exposure if previously configured
    sed -i '/0\.0\.0\.0\/0/d' "${CONF_DIR}/pg_hba.conf"
    sed -i '/::0\/0/d' "${CONF_DIR}/pg_hba.conf"

    # Ensure secure loopback SCRAM-SHA-256 entries exist
    if ! grep -q "127.0.0.1/32.*scram-sha-256" "${CONF_DIR}/pg_hba.conf"; then
        echo "host    all             all             127.0.0.1/32            scram-sha-256" >> "${CONF_DIR}/pg_hba.conf"
        echo "host    all             all             ::1/128                 scram-sha-256" >> "${CONF_DIR}/pg_hba.conf"
    fi
fi

systemctl restart postgresql

echo "[INFO] Creating database '${DB_NAME}' if not exists..."
su - postgres -c "psql -tc \"SELECT 1 FROM pg_database WHERE datname = '${DB_NAME}'\" | grep -q 1 || psql -c \"CREATE DATABASE ${DB_NAME};\""

echo "[INFO] Creating or configuring least-privilege application role '${APP_USER}'..."
# Ensure user exists without SUPERUSER or CREATEDB privileges
su - postgres -c "psql -tc \"SELECT 1 FROM pg_roles WHERE rolname = '${APP_USER}'\" | grep -q 1 || psql -c \"CREATE USER ${APP_USER} WITH PASSWORD '${APP_PASSWORD}' NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;\""

# Enforce NOSUPERUSER NOCREATEDB NOCREATEROLE on existing role
su - postgres -c "psql -c \"ALTER USER ${APP_USER} WITH PASSWORD '${APP_PASSWORD}' NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;\""

echo "[INFO] Applying least-privilege grants on '${DB_NAME}'..."
su - postgres -c "psql -d ${DB_NAME} <<EOF
-- Connect privilege
GRANT CONNECT ON DATABASE ${DB_NAME} TO ${APP_USER};

-- Schema usage and table creation for migrations
GRANT USAGE, CREATE ON SCHEMA public TO ${APP_USER};

-- DML permissions on existing and future tables
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO ${APP_USER};
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO ${APP_USER};

-- Sequence permissions for auto-increment IDs
GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO ${APP_USER};
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT USAGE, SELECT, UPDATE ON SEQUENCES TO ${APP_USER};
EOF"

echo "POSTGRESQL_SETUP_SUCCESS"
