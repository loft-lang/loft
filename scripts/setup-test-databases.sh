#!/usr/bin/env bash
# Create the PostgreSQL and MariaDB test databases the sqldb fixture expects.
#
# These are the two backends of the local four-backend bar that need a SERVER
# (doc/claude/TEST_ENVIRONMENTS.md § Database backends).  CI never runs them — it gates
# sqlite only — so this exists to make a developer box reproducible, and to say
# what the setup actually is rather than leaving it as folklore on one machine.
#
# It is IDEMPOTENT and it never drops anything: every step checks first and
# skips what is already there.
#
# Needs administrative access to each server (`sudo -u postgres`, `sudo mysql`).
# Run the two halves separately with --pg / --maria if only one applies.
#
# Usage:  scripts/setup-test-databases.sh [--pg | --maria]
set -euo pipefail

PG_DB="loft_test_pg"
MY_USER="loft"
MY_PASS="loft"          # local test credential only — it is the fixture's default
MY_DB="loft_test_uni"
MY_SCOPE='loft\_test%'  # the grant pattern: everything named loft_test*, nothing else

want="${1:---all}"
did_any=0

# ── PostgreSQL ────────────────────────────────────────────────────────────────
# The fixture connects as `dbname=loft_test_pg` with no user and no host, which
# means a unix-socket peer connection AS THE OS USER.  So the role to create is
# the person running the tests, not a shared `loft` role.
if [ "$want" = "--all" ] || [ "$want" = "--pg" ]; then
    did_any=1
    if ! command -v psql >/dev/null; then
        echo "psql not found — install postgresql-client (and a server) first." >&2
    else
        me=$(id -un)
        echo "PostgreSQL: ensuring role '$me' and database '$PG_DB' …"
        # Check WITHOUT sudo first: on a box that is already set up this whole
        # script is a verifier, and a verifier that demands admin does not get
        # run.  Escalate only when something is actually missing.
        if psql -d postgres -tAc "select 1 from pg_roles where rolname='$me'" 2>/dev/null | grep -q 1; then
            echo "  role $me already exists"
        else
            sudo -u postgres createuser --createdb "$me"
            echo "  created role $me (CREATEDB, not superuser)"
        fi
        if ! psql -tAc "select 1 from pg_database where datname='$PG_DB'" postgres | grep -q 1; then
            createdb "$PG_DB"
            echo "  created database $PG_DB"
        else
            echo "  database $PG_DB already exists"
        fi
        psql -tAc "select 'ok ' || current_user || ' @ ' || current_database()" -d "$PG_DB"
    fi
fi

# ── MariaDB ───────────────────────────────────────────────────────────────────
# The user is SCOPED to loft_test* on purpose: a suite that can drop a
# developer's other schemas is one bad DROP away from being a very bad day.
# Anything outside the pattern answers ERROR 1044.
if [ "$want" = "--all" ] || [ "$want" = "--maria" ]; then
    did_any=1
    if ! command -v mysql >/dev/null; then
        echo "mysql client not found — install mariadb-client (and a server) first." >&2
    else
        echo "MariaDB: ensuring user '$MY_USER'@'localhost' and database '$MY_DB' …"
        if mysql -h 127.0.0.1 -u "$MY_USER" -p"$MY_PASS" "$MY_DB" -e "select 1" >/dev/null 2>&1; then
            echo "  user and database already usable — skipping the privileged step"
        else
        sudo mysql <<SQL
CREATE USER IF NOT EXISTS '${MY_USER}'@'localhost' IDENTIFIED BY '${MY_PASS}';
CREATE USER IF NOT EXISTS '${MY_USER}'@'127.0.0.1' IDENTIFIED BY '${MY_PASS}';
GRANT ALL PRIVILEGES ON \`${MY_SCOPE}\`.* TO '${MY_USER}'@'localhost';
GRANT ALL PRIVILEGES ON \`${MY_SCOPE}\`.* TO '${MY_USER}'@'127.0.0.1';
CREATE DATABASE IF NOT EXISTS ${MY_DB};
FLUSH PRIVILEGES;
SQL
        echo "  granted ALL on ${MY_SCOPE} only"
        fi
        # Prove the scope actually bites, rather than trusting the GRANT text.
        if mysql -h 127.0.0.1 -u "$MY_USER" -p"$MY_PASS" \
                 -e "CREATE DATABASE loft_setup_scope_probe" >/dev/null 2>&1; then
            echo "  WARNING: the user created a database OUTSIDE loft_test* — the scope is not enforced." >&2
            mysql -h 127.0.0.1 -u "$MY_USER" -p"$MY_PASS" -e "DROP DATABASE loft_setup_scope_probe" >/dev/null 2>&1 || true
        else
            echo "  scope verified: a database outside loft_test* is refused"
        fi
        mysql -N -B -h 127.0.0.1 -u "$MY_USER" -p"$MY_PASS" "$MY_DB" -e "select concat('ok ', current_user(), ' @ ', database())" 2>/dev/null
    fi
fi

[ "$did_any" = "1" ] || { echo "unknown option: $want (use --pg, --maria, or nothing)" >&2; exit 2; }

cat <<'DONE'

Check the whole local bar with:

  LOFT_SQLDB_MODE=sqlite   target/release/loft --interpret --lib tests/fixtures/sqldb tests/fixtures/sqldb/uniform.loft
  LOFT_SQLDB_MODE=postgres …
  LOFT_SQLDB_MODE=maria    …
  LD_LIBRARY_PATH=~/.local/lib LOFT_SQLDB_MODE=duckdb …

A backend that is not reachable prints SKIP; a skip is never a pass.
DONE
