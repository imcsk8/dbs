#!/usr/bin/env bash
# ==============================================================================
# DBS Database Provisioning & Bootstrap Script
# ==============================================================================
# Automates PostgreSQL database provisioning for both:
#   1. Native Host PostgreSQL (dnf install postgresql-server / systemd)
#   2. Container PostgreSQL (Podman / Docker)
#
# Bootstraps the DBS supply-chain database schema natively using:
#   dbs db bootstrap
# ==============================================================================

set -euo pipefail

# Default configuration parameters
DB_USER="${DB_USER:-dbs}"
DB_PASSWORD="${DB_PASSWORD:-prueba123}"
DB_NAME="${DB_NAME:-dbs}"
DB_HOST="${DB_HOST:-127.0.0.1}"
DB_PORT="${DB_PORT:-5432}"
MODE="${MODE:-host}" # 'host' or 'container'
DB_CONTAINER="${DB_CONTAINER:-dbs_pg}"
CONTAINER_PORT="${CONTAINER_PORT:-9436}"
DATA_DIR="${DATA_DIR:-./data}"
DBS_BIN="${DBS_BIN:-./bin/dbs}"
FORCE=false

# ANSI Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

log_info()    { echo -e "${CYAN}${BOLD}[INFO]${NC} $*"; }
log_step()    { echo -e "\n${BLUE}${BOLD}==>${NC} ${BOLD}$*${NC}"; }
log_success() { echo -e "${GREEN}${BOLD}[SUCCESS]${NC} $*"; }
log_warn()    { echo -e "${YELLOW}${BOLD}[WARNING]${NC} $*"; }
log_error()   { echo -e "${RED}${BOLD}[ERROR]${NC} $*" >&2; }

usage() {
    cat <<EOF
Usage: $0 [OPTIONS]

Provisions PostgreSQL and bootstraps the DBS supply-chain database schema.

Options:
  --mode <host|container>   Deployment mode (default: host)
  --user <USER>             Database user (default: dbs)
  --password <PASS>         Database password (default: prueba123)
  --name <DBNAME>           Database name (default: dbs)
  --host <HOST>             Database host for native mode (default: 127.0.0.1)
  --port <PORT>             Database port (default: 5432 for host, 9436 for container)
  --dbs-bin <PATH>          Path to dbs binary (default: ./bin/dbs or 'dbs' from PATH)
  -f, --force               Force schema re-initialization (drops existing tables)
  -h, --help                Show this help message

Examples:
  # Native host PostgreSQL setup (Fedora / ELN / RHEL / TacOS):
  sudo $0 --mode host

  # Container PostgreSQL setup (Podman):
  $0 --mode container

  # Re-bootstrap existing database:
  $0 --mode host --force
EOF
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --mode)
            MODE="$2"
            shift 2
            ;;
        --user)
            DB_USER="$2"
            shift 2
            ;;
        --password)
            DB_PASSWORD="$2"
            shift 2
            ;;
        --name)
            DB_NAME="$2"
            shift 2
            ;;
        --host)
            DB_HOST="$2"
            shift 2
            ;;
        --port)
            DB_PORT="$2"
            shift 2
            ;;
        --dbs-bin)
            DBS_BIN="$2"
            shift 2
            ;;
        -f|--force)
            FORCE=true
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            log_error "Unknown option: $1"
            usage
            exit 1
            ;;
    esac
done

# Resolve DBS binary
if [[ ! -x "${DBS_BIN}" ]]; then
    if command -v dbs >/dev/null 2>&1; then
        DBS_BIN="$(command -v dbs)"
    else
        log_error "dbs binary not found at '${DBS_BIN}' or in PATH. Build it first with 'make release'."
        exit 1
    fi
fi

# ------------------------------------------------------------------------------
# 1. Host Native PostgreSQL Provisioning
# ------------------------------------------------------------------------------
setup_host_postgresql() {
    log_step "Setting up Native Host PostgreSQL..."

    if ! command -v systemctl >/dev/null 2>&1; then
        log_error "systemctl not found. Host mode requires systemd."
        exit 1
    fi

    # Check if postgresql server packages are installed
    if ! rpm -q postgresql-server >/dev/null 2>&1 && ! dpkg -s postgresql >/dev/null 2>&1; then
        log_warn "PostgreSQL server package is not installed."
        if command -v dnf >/dev/null 2>&1; then
            log_info "Installing postgresql-server via dnf..."
            sudo dnf install -y postgresql-server postgresql-contrib
        elif command -v apt-get >/dev/null 2>&1; then
            log_info "Installing postgresql via apt-get..."
            sudo apt-get update && sudo apt-get install -y postgresql postgresql-contrib
        else
            log_error "Please install postgresql-server manually on your distribution."
            exit 1
        fi
    fi

    # Initialize PostgreSQL data directory if needed (RHEL/Fedora/ELN/TacOS)
    if command -v postgresql-setup >/dev/null 2>&1; then
        if [[ ! -f /var/lib/pgsql/data/PG_VERSION ]]; then
            log_info "Initializing PostgreSQL database cluster with postgresql-setup --initdb..."
            sudo postgresql-setup --initdb
        fi
    fi

    # Apply High-Throughput Tuning to postgresql.conf if not already present
    local PG_CONF="/var/lib/pgsql/data/postgresql.conf"
    if [[ -f "${PG_CONF}" ]]; then
        if ! grep -q "DBS High-Throughput Tuning" "${PG_CONF}"; then
            log_info "Appending high-throughput tuning to ${PG_CONF}..."
            sudo tee -a "${PG_CONF}" >/dev/null << 'EOF'

# --- DBS High-Throughput Tuning ---
synchronous_commit = off
shared_buffers = 512MB
work_mem = 64MB
temp_buffers = 32MB
EOF
        fi
    fi

    # Configure pg_hba.conf for local md5/scram/trust access for dbs user
    local PG_HBA="/var/lib/pgsql/data/pg_hba.conf"
    if [[ -f "${PG_HBA}" ]]; then
        if ! grep -q "dbs.*${DB_USER}" "${PG_HBA}"; then
            log_info "Allowing local connections for '${DB_USER}' in ${PG_HBA}..."
            sudo sed -i "1i local   ${DB_NAME}     ${DB_USER}                                md5" "${PG_HBA}"
            sudo sed -i "2i host    ${DB_NAME}     ${DB_USER}        127.0.0.1/32            md5" "${PG_HBA}"
            sudo sed -i "3i host    ${DB_NAME}     ${DB_USER}        ::1/128                 md5" "${PG_HBA}"
        fi
    fi

    # Enable and start postgresql service
    log_info "Starting and enabling postgresql.service..."
    sudo systemctl enable --now postgresql
    sudo systemctl reload-or-restart postgresql

    # Create role and database as postgres user
    log_info "Creating database role '${DB_USER}' and database '${DB_NAME}'..."
    sudo -u postgres psql -tc "SELECT 1 FROM pg_roles WHERE rolname='${DB_USER}'" | grep -q 1 || \
        sudo -u postgres psql -c "CREATE USER ${DB_USER} WITH PASSWORD '${DB_PASSWORD}';"

    sudo -u postgres psql -tc "SELECT 1 FROM pg_database WHERE datname='${DB_NAME}'" | grep -q 1 || \
        sudo -u postgres psql -c "CREATE DATABASE ${DB_NAME} OWNER ${DB_USER};"

    sudo -u postgres psql -c "GRANT ALL PRIVILEGES ON DATABASE ${DB_NAME} TO ${DB_USER};"
    sudo -u postgres psql -d "${DB_NAME}" -c "GRANT ALL ON SCHEMA public TO ${DB_USER};"

    TARGET_URL="postgres://${DB_USER}:${DB_PASSWORD}@${DB_HOST}:${DB_PORT}/${DB_NAME}"
}

# ------------------------------------------------------------------------------
# 2. Container PostgreSQL Provisioning
# ------------------------------------------------------------------------------
setup_container_postgresql() {
    log_step "Setting up Container PostgreSQL..."

    local RUNTIME=""
    if command -v podman >/dev/null 2>&1; then
        RUNTIME="podman"
    elif command -v docker >/dev/null 2>&1; then
        RUNTIME="docker"
    else
        log_error "Neither podman nor docker found for container mode."
        exit 1
    fi

    mkdir -p "${DATA_DIR}"

    log_info "Starting high-throughput PostgreSQL container via ${RUNTIME}..."
    ${RUNTIME} run -d --replace --name="${DB_CONTAINER}" \
        -e POSTGRES_PASSWORD="${DB_PASSWORD}" \
        -e POSTGRES_USER="${DB_USER}" \
        -e POSTGRES_DB="${DB_NAME}" \
        -e POSTGRES_INITDB_ARGS="--encoding UTF-8" \
        -v "${DATA_DIR}:/var/lib/postgresql/data:U,Z" \
        -p "127.0.0.1:${CONTAINER_PORT}:5432" ghcr.io/enterprisedb/postgresql:17 \
        -c shared_buffers=512MB \
        -c work_mem=64MB \
        -c synchronous_commit=off \
        -c temp_buffers=32MB

    log_info "Waiting for PostgreSQL container to accept connections..."
    sleep 3

    TARGET_URL="postgres://${DB_USER}:${DB_PASSWORD}@127.0.0.1:${CONTAINER_PORT}/${DB_NAME}"
}

# ------------------------------------------------------------------------------
# Execution Flow
# ------------------------------------------------------------------------------
TARGET_URL=""

if [[ "${MODE}" == "host" ]]; then
    setup_host_postgresql
elif [[ "${MODE}" == "container" ]]; then
    setup_container_postgresql
else
    log_error "Invalid mode: ${MODE}. Choose 'host' or 'container'."
    exit 1
fi

# ------------------------------------------------------------------------------
# 3. Write Environment Configuration (.env & /etc/dbs/dbs.env)
# ------------------------------------------------------------------------------
log_step "Writing Environment Configurations..."
echo "DATABASE_URL=\"${TARGET_URL}\"" > .env
log_success "Updated local .env"

if [[ -d /etc/dbs ]] || [[ $EUID -eq 0 ]] || sudo -n true 2>/dev/null; then
    sudo mkdir -p /etc/dbs
    echo "DATABASE_URL=\"${TARGET_URL}\"" | sudo tee /etc/dbs/dbs.env >/dev/null
    sudo chmod 640 /etc/dbs/dbs.env
    log_success "Updated /etc/dbs/dbs.env"
fi

# ------------------------------------------------------------------------------
# 4. Bootstrap DBS Schema via Embedded Binary
# ------------------------------------------------------------------------------
log_step "Bootstrapping DBS Database Schema..."
export DATABASE_URL="${TARGET_URL}"

if [[ "${FORCE}" == "true" ]]; then
    log_info "Running: ${DBS_BIN} db reset --force"
    "${DBS_BIN}" db reset --force
else
    log_info "Running: ${DBS_BIN} db bootstrap"
    "${DBS_BIN}" db bootstrap || {
        log_warn "Bootstrap skipped (already initialized). Run with --force to recreate."
    }
fi

# ------------------------------------------------------------------------------
# 5. Verify Database Status
# ------------------------------------------------------------------------------
log_step "Verifying Database Status..."
"${DBS_BIN}" db status

log_success "DBS database is fully provisioned, tuned, and ready for builds!"
EOF
