#!/usr/bin/env bash
set -euo pipefail

# ==============================================================================
# Spine Forge - Secure PostgreSQL Setup (WSL / Local Linux)
# Least Privilege & Loopback Network Binding Enforcement
# ==============================================================================

POSTGRES_VERSION="18"
CONF_DIR="/etc/postgresql/${POSTGRES_VERSION}/main"
DB_NAME="${SPINE_POSTGRES_DB:-spine_forge}"
TEST_DB_NAME="${SPINE_TEST_POSTGRES_DB:-spine_forge_test}"
APP_USER="${SPINE_POSTGRES_USER:-spine_app}"
APP_PASSWORD="${SPINE_POSTGRES_PASSWORD:-spine_secure_dev_password}"

# Detect WSL internal IPv4 address if running in WSL
WSL_IP=$(ip -4 addr show eth0 2>/dev/null | grep -oP '(?<=inet\s)\d+(\.\d+){3}' || true)
if [ -n "${WSL_IP}" ]; then
    LISTEN_VAL="127.0.0.1,${WSL_IP}"
else
    LISTEN_VAL="127.0.0.1"
fi

echo "[INFO] Configuring PostgreSQL network binding to loopback and private WSL interface (${LISTEN_VAL})..."
if [ -f "${CONF_DIR}/postgresql.conf" ]; then
    sed -i "s/^#\?listen_addresses\s*=.*/listen_addresses = '${LISTEN_VAL}'/" "${CONF_DIR}/postgresql.conf"
fi

# Configure host-based authentication (pg_hba.conf) with least privilege
# Disallow 0.0.0.0/0 and ::0/0 broad network exposure. Only permit loopback and private host virtual subnet.
if [ -f "${CONF_DIR}/pg_hba.conf" ]; then
    sed -i '/0\.0\.0\.0\/0/d' "${CONF_DIR}/pg_hba.conf"
    sed -i '/::0\/0/d' "${CONF_DIR}/pg_hba.conf"

    if ! grep -q "127.0.0.1/32.*scram-sha-256" "${CONF_DIR}/pg_hba.conf"; then
        echo "host    all             all             127.0.0.1/32            scram-sha-256" >> "${CONF_DIR}/pg_hba.conf"
        echo "host    all             all             ::1/128                 scram-sha-256" >> "${CONF_DIR}/pg_hba.conf"
    fi

    if [ -n "${WSL_IP}" ] && ! grep -q "172.16.0.0/12.*scram-sha-256" "${CONF_DIR}/pg_hba.conf"; then
        echo "host    all             all             172.16.0.0/12           scram-sha-256" >> "${CONF_DIR}/pg_hba.conf"
    fi
fi

systemctl restart postgresql

echo "[INFO] Creating database '${DB_NAME}' if not exists..."
su - postgres -c "psql -tc \"SELECT 1 FROM pg_database WHERE datname = '${DB_NAME}'\" | grep -q 1 || psql -c \"CREATE DATABASE ${DB_NAME};\""

echo "[INFO] Creating dedicated test database '${TEST_DB_NAME}' if not exists..."
su - postgres -c "psql -tc \"SELECT 1 FROM pg_database WHERE datname = '${TEST_DB_NAME}'\" | grep -q 1 || psql -c \"CREATE DATABASE ${TEST_DB_NAME};\""

echo "[INFO] Creating or configuring least-privilege application role '${APP_USER}'..."
# Ensure user exists without SUPERUSER, CREATEDB, CREATEROLE, or BYPASSRLS privileges
su - postgres -c "psql -tc \"SELECT 1 FROM pg_roles WHERE rolname = '${APP_USER}'\" | grep -q 1 || psql -c \"CREATE USER ${APP_USER} WITH PASSWORD '${APP_PASSWORD}' NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;\""
su - postgres -c "psql -c \"ALTER USER ${APP_USER} WITH PASSWORD '${APP_PASSWORD}' NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;\""

echo "[INFO] Applying least-privilege grants on '${DB_NAME}'..."
su - postgres -c "psql -d ${DB_NAME} <<EOF
GRANT CONNECT ON DATABASE ${DB_NAME} TO ${APP_USER};
GRANT USAGE, CREATE ON SCHEMA public TO ${APP_USER};
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO ${APP_USER};
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO ${APP_USER};
GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO ${APP_USER};
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT USAGE, SELECT, UPDATE ON SEQUENCES TO ${APP_USER};
EOF"

echo "[INFO] Applying least-privilege grants on '${TEST_DB_NAME}'..."
su - postgres -c "psql -d ${TEST_DB_NAME} <<EOF
GRANT CONNECT ON DATABASE ${TEST_DB_NAME} TO ${APP_USER};
GRANT USAGE, CREATE ON SCHEMA public TO ${APP_USER};
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO ${APP_USER};
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO ${APP_USER};
GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO ${APP_USER};
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT USAGE, SELECT, UPDATE ON SEQUENCES TO ${APP_USER};
EOF"

echo "POSTGRESQL_SETUP_SUCCESS"
