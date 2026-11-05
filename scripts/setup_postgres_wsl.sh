#!/usr/bin/env bash
set -e

# Configure listen_addresses
sed -i "s/#listen_addresses = 'localhost'/listen_addresses = '*'/" /etc/postgresql/18/main/postgresql.conf

# Configure pg_hba.conf
if ! grep -q "0.0.0.0/0" /etc/postgresql/18/main/pg_hba.conf; then
    echo "host all all 0.0.0.0/0 scram-sha-256" >> /etc/postgresql/18/main/pg_hba.conf
    echo "host all all ::0/0 scram-sha-256" >> /etc/postgresql/18/main/pg_hba.conf
fi

systemctl restart postgresql

# Create role spine_app and database spine_forge
su - postgres -c "psql -c \"CREATE USER spine_app WITH PASSWORD 'spine_secure_dev_password' SUPERUSER CREATEDB;\"" || true
su - postgres -c "psql -c \"ALTER USER spine_app WITH PASSWORD 'spine_secure_dev_password';\""
su - postgres -c "createdb -O spine_app spine_forge" || true

echo "POSTGRESQL_SETUP_SUCCESS"
