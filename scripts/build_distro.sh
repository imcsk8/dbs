#!/usr/bin/env bash
# ==============================================================================
# TacOS Distribution Repository Build & Publication Script
# ==============================================================================
# Automates the end-to-end lifecycle for building a TacOS distribution repository:
#   1. Validates host kernel tuning and permissions
#   2. Prepares directory hierarchies and BTRFS lookaside cache
#   3. Syncs upstream Fedora Rawhide dist-git repositories
#   4. Pre-stages source archives via BTRFS CoW reflinks
#   5. Executes topological DAG layered builds with dynamic repo feedback
#   6. Organizes repository layout, runs createrepo_c, and generates client .repo
#   7. Configures and validates Nginx distribution exposure
# ==============================================================================

set -euo pipefail

# Default configuration parameters
DISTRO_NAME="tacos-stable-x86_64"
CHROOT_PROFILE="tacos-stable-x86_64"
DISTGIT_DIR="/srv/dbs/tacos/rpm"
DISTRO_ROOT="/srv/dbs/tacos/distro"
LOOKASIDE_DIR="/srv/dbs/lookaside"
STAGING_DIR="/srv/dbs/tacos/staging"
CONCURRENCY=4
NGINX_CONF_DIR="/etc/nginx/conf.d"
DBS_BIN="./bin/dbs"

# Feature flags
SETUP_HOST=false
CLONE_DISTGIT=false
CONFIGURE_NGINX=false
SKIP_BUILD=false
SIGN_PACKAGES=false
GPG_KEY_ID=""

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
Usage: $(basename "$0") [OPTIONS]

Options:
  -n, --name <NAME>           Distribution name [default: tacos-stable-x86_64]
  -c, --chroot <PROFILE>      Mock chroot profile [default: tacos-stable-x86_64]
  -s, --distgit-dir <PATH>    Directory containing dist-git repos [default: /srv/dbs/tacos/rpm]
  -d, --distro-dir <PATH>     Distro repository root [default: /srv/dbs/tacos/distro]
  -l, --lookaside <PATH>      Lookaside cache directory [default: /srv/dbs/lookaside]
  -j, --concurrency <NUM>     Worker concurrency level [default: 4]
      --setup-host            Apply recommended host kernel tuning (sysctl)
      --clone-distgit         Clone/sync Fedora Rawhide dist-git repositories
      --configure-nginx       Generate and install Nginx vhost config
      --sign <KEY_ID>         GPG sign RPM packages and repository metadata
      --skip-build            Skip compilation phase (only sync / setup / publish)
  -h, --help                  Display this help message
EOF
    exit 0
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        -n|--name) DISTRO_NAME="$2"; shift 2 ;;
        -c|--chroot) CHROOT_PROFILE="$2"; shift 2 ;;
        -s|--distgit-dir) DISTGIT_DIR="$2"; shift 2 ;;
        -d|--distro-dir) DISTRO_ROOT="$2"; shift 2 ;;
        -l|--lookaside) LOOKASIDE_DIR="$2"; shift 2 ;;
        -j|--concurrency) CONCURRENCY="$2"; shift 2 ;;
        --setup-host) SETUP_HOST=true; shift ;;
        --clone-distgit) CLONE_DISTGIT=true; shift ;;
        --configure-nginx) CONFIGURE_NGINX=true; shift ;;
        --sign) SIGN_PACKAGES=true; GPG_KEY_ID="$2"; shift 2 ;;
        --skip-build) SKIP_BUILD=true; shift ;;
        -h|--help) usage ;;
        *) log_error "Unknown argument: $1"; usage ;;
    esac
done

TARGET_DISTRO_DIR="${DISTRO_ROOT}/${DISTRO_NAME}"
RPM_REPO_DIR="${TARGET_DISTRO_DIR}/x86_64"
SRPM_REPO_DIR="${TARGET_DISTRO_DIR}/source/SRPMS"

# ------------------------------------------------------------------------------
# Phase 1: Environment & Tool Verification
# ------------------------------------------------------------------------------
log_step "Phase 1: Verifying tools and environment"

command -v mock >/dev/null 2>&1 || { log_error "Mock is not installed. Run: sudo dnf install -y mock"; exit 1; }
command -v createrepo_c >/dev/null 2>&1 || { log_error "createrepo_c is not installed. Run: sudo dnf install -y createrepo_c"; exit 1; }

if [[ ! -x "${DBS_BIN}" ]]; then
    if [[ -x "./rust/bin/dbs" ]]; then
        DBS_BIN="./rust/bin/dbs"
    elif command -v dbs >/dev/null 2>&1; then
        DBS_BIN="$(command -v dbs)"
    else
        log_warn "DBS binary not found at ${DBS_BIN}. Compiling release binary..."
        make release
        DBS_BIN="./bin/dbs"
    fi
fi
log_info "Using DBS binary: ${DBS_BIN}"

# Check mock group membership
if ! groups | grep -qw "mock"; then
    log_warn "Current user is not in the 'mock' group. Builds may require passwordless sudo for mock."
fi

# ------------------------------------------------------------------------------
# Phase 2: Host Kernel Tuning (Optional)
# ------------------------------------------------------------------------------
if [[ "${SETUP_HOST}" == true ]]; then
    log_step "Phase 2: Applying Host Kernel Tuning"
    SYSCTL_CONF="config/sysctl/99-dbs-build-host.conf"
    if [[ -f "${SYSCTL_CONF}" ]]; then
        log_info "Installing ${SYSCTL_CONF} to /etc/sysctl.d/..."
        sudo cp "${SYSCTL_CONF}" /etc/sysctl.d/99-dbs-build-host.conf
        sudo sysctl --system >/dev/null
        log_success "Host kernel tuning active: fs.pipe-user-pages-soft=$(cat /proc/sys/fs/pipe-user-pages-soft)"
    else
        log_warn "Sysctl file ${SYSCTL_CONF} not found, applying inline..."
        sudo sysctl -w fs.pipe-user-pages-soft=0
        sudo sysctl -w fs.pipe-user-pages-hard=0
        sudo sysctl -w fs.file-max=2097152
    fi
else
    # Check if pipe soft limit is zero
    CURRENT_SOFT=$(cat /proc/sys/fs/pipe-user-pages-soft 2>/dev/null || echo "16384")
    if [[ "${CURRENT_SOFT}" != "0" ]]; then
        log_warn "fs.pipe-user-pages-soft is ${CURRENT_SOFT} (recommended: 0). Run with --setup-host to tune."
    fi
fi

# ------------------------------------------------------------------------------
# Phase 3: Directory Hierarchy & Storage Setup
# ------------------------------------------------------------------------------
log_step "Phase 3: Initializing directory hierarchy"

sudo mkdir -p "${DISTGIT_DIR}" "${LOOKASIDE_DIR}" "${STAGING_DIR}" "${RPM_REPO_DIR}" "${SRPM_REPO_DIR}"
sudo chown -R "${USER}:mock" /srv/dbs 2>/dev/null || sudo chown -R "${USER}:${USER}" /srv/dbs
sudo chmod -R 775 /srv/dbs 2>/dev/null || true

log_info "Dist-git spec root:  ${DISTGIT_DIR}"
log_info "Lookaside cache:      ${LOOKASIDE_DIR}"
log_info "Build staging:        ${STAGING_DIR}"
log_info "Distribution output:  ${TARGET_DISTRO_DIR}"

# ------------------------------------------------------------------------------
# Phase 4: Mock Chroot Configuration
# ------------------------------------------------------------------------------
log_step "Phase 4: Ensuring Mock chroot configuration"

CHROOT_CFG="mock/${CHROOT_PROFILE}.cfg"
if [[ ! -f "${CHROOT_CFG}" && ! -f "/etc/mock/${CHROOT_PROFILE}.cfg" ]]; then
    log_info "Generating Mock configuration: ${CHROOT_CFG}"
    mkdir -p mock
    cat <<EOF > "${CHROOT_CFG}"
config_opts['target_arch'] = 'x86_64'
config_opts['legal_host_arches'] = ('x86_64',)
include('templates/tacos-rolling.tpl')
EOF
    log_success "Created ${CHROOT_CFG}"
fi

# ------------------------------------------------------------------------------
# Phase 5: Ingest Dist-Git Repositories (Optional)
# ------------------------------------------------------------------------------
if [[ "${CLONE_DISTGIT}" == true ]]; then
    log_step "Phase 5: Synchronizing dist-git repositories from Fedora Rawhide"
    log_info "Syncing dist-git repos into ${DISTGIT_DIR}..."
    "${DBS_BIN}" distgit sync \
        --distro fedora-rawhide \
        --dest "${DISTGIT_DIR}" \
        --concurrency "${CONCURRENCY}" \
        --lookaside-dir "${LOOKASIDE_DIR}" \
        --sources
    log_success "Dist-git repositories synchronized."
fi

# ------------------------------------------------------------------------------
# Phase 6: Topological Compilation (DAG Build)
# ------------------------------------------------------------------------------
if [[ "${SKIP_BUILD}" == false ]]; then
    log_step "Phase 6: Computing DAG and executing layered parallel builds"
    
    SPEC_COUNT=$(find "${DISTGIT_DIR}" -maxdepth 3 -name "*.spec" | wc -l)
    if [[ "${SPEC_COUNT}" -eq 0 ]]; then
        log_error "No .spec files found in ${DISTGIT_DIR}. Clone or add packages first."
        exit 1
    fi
    log_info "Discovered ${SPEC_COUNT} package specification(s) in ${DISTGIT_DIR}"

    "${DBS_BIN}" dag \
        --path "${DISTGIT_DIR}" \
        --mock-root "${CHROOT_PROFILE}" \
        --mock-config-dir "mock" \
        --lookaside-dir "${LOOKASIDE_DIR}" \
        --output-dir "${STAGING_DIR}" \
        --concurrency "${CONCURRENCY}" \
        --fetch-sources \
        --dynamic-repo \
        --build
    log_success "Topological DAG build execution complete."
fi

# ------------------------------------------------------------------------------
# Phase 7: Repository Publication & Metadata Indexing
# ------------------------------------------------------------------------------
log_step "Phase 7: Organizing distribution repository and generating repodata"

# Collect binary and source RPMs from staging
STAGING_RPMS="${STAGING_DIR}/rpms/x86_64"
if [[ -d "${STAGING_RPMS}" ]]; then
    log_info "Moving built binary RPMs to ${RPM_REPO_DIR}..."
    find "${STAGING_RPMS}" -name "*.rpm" ! -name "*.src.rpm" -exec cp -u {} "${RPM_REPO_DIR}/" \; 2>/dev/null || true
    
    log_info "Moving source RPMs to ${SRPM_REPO_DIR}..."
    find "${STAGING_RPMS}" -name "*.src.rpm" -exec cp -u {} "${SRPM_REPO_DIR}/" \; 2>/dev/null || true
fi

# Check for packages in any worker output directories
find "${STAGING_DIR}" -name "*.rpm" ! -name "*.src.rpm" -exec cp -u {} "${RPM_REPO_DIR}/" \; 2>/dev/null || true
find "${STAGING_DIR}" -name "*.src.rpm" -exec cp -u {} "${SRPM_REPO_DIR}/" \; 2>/dev/null || true

# Optional GPG signing
if [[ "${SIGN_PACKAGES}" == true && -n "${GPG_KEY_ID}" ]]; then
    log_info "Signing binary RPM packages with GPG key: ${GPG_KEY_ID}"
    rpm --addsign --key-id="${GPG_KEY_ID}" "${RPM_REPO_DIR}"/*.rpm || log_warn "GPG signing skipped or failed."
fi

# Generate / update repository metadata
log_info "Indexing repository metadata with createrepo_c..."
createrepo_c --update --workers "${CONCURRENCY}" "${RPM_REPO_DIR}"
if [[ -d "${SRPM_REPO_DIR}" ]] && compgen -G "${SRPM_REPO_DIR}/*.src.rpm" > /dev/null; then
    createrepo_c --update --workers "${CONCURRENCY}" "${SRPM_REPO_DIR}"
fi
log_success "Repository metadata generated at ${RPM_REPO_DIR}/repodata/"

# ------------------------------------------------------------------------------
# Phase 8: Generate Client .repo File
# ------------------------------------------------------------------------------
log_step "Phase 8: Generating client repository configuration"

CLIENT_REPO_FILE="${TARGET_DISTRO_DIR}/${DISTRO_NAME}.repo"
cat <<EOF > "${CLIENT_REPO_FILE}"
[${DISTRO_NAME}]
name=TacOS Linux - ${DISTRO_NAME} (\$basearch)
baseurl=http://repos.tacos.org.mx/${DISTRO_NAME}/\$basearch
enabled=1
gpgcheck=0
metadata_expire=300
skip_if_unavailable=False

[${DISTRO_NAME}-source]
name=TacOS Linux - ${DISTRO_NAME} (Source)
baseurl=http://repos.tacos.org.mx/${DISTRO_NAME}/source/SRPMS
enabled=0
gpgcheck=0
metadata_expire=300
skip_if_unavailable=False
EOF
log_success "Client repo configuration written to: ${CLIENT_REPO_FILE}"

# ------------------------------------------------------------------------------
# Phase 9: Nginx Virtual Host Configuration (Optional)
# ------------------------------------------------------------------------------
if [[ "${CONFIGURE_NGINX}" == true ]]; then
    log_step "Phase 9: Configuring Nginx virtual host"
    
    NGINX_CONF="${NGINX_CONF_DIR}/tacos-distro.conf"
    log_info "Generating Nginx configuration at ${NGINX_CONF}..."

    sudo tee "${NGINX_CONF}" > /dev/null <<EOF
# TacOS Distribution Repository Web Server
server {
    listen 80;
    server_name repos.tacos.org.mx;

    root ${DISTRO_ROOT};

    # Enable directory browsing
    autoindex on;
    autoindex_exact_size off;
    autoindex_localtime on;

    # Performance streaming for binary packages
    sendfile on;
    tcp_nopush on;
    tcp_nodelay on;

    # Metadata must never be cached so clients fetch fresh package lists
    location ~* /repodata/.*$ {
        expires -1;
        add_header Cache-Control "no-cache, no-store, must-revalidate";
    }

    # Immutable binary RPMs can be cached
    location ~* \.rpm$ {
        expires 30d;
        add_header Cache-Control "public";
    }

    # Logging
    access_log /var/log/nginx/tacos_distro_access.log;
    error_log  /var/log/nginx/tacos_distro_error.log;
}
EOF

    if command -v nginx >/dev/null 2>&1; then
        log_info "Testing Nginx configuration syntax..."
        sudo nginx -t && sudo systemctl reload nginx || log_warn "Please start or reload Nginx manually."
        log_success "Nginx repository server configured and reloaded."
    else
        log_warn "Nginx binary not found. Configuration written to ${NGINX_CONF}."
    fi
fi

# ------------------------------------------------------------------------------
# Summary
# ------------------------------------------------------------------------------
echo -e "\n==========================================================="
echo -e " ${GREEN}${BOLD}TacOS Distribution Repository Successfully Prepared!${NC}"
echo -e "==========================================================="
echo -e " * Repository Name:    ${BOLD}${DISTRO_NAME}${NC}"
echo -e " * Base Directory:     ${BOLD}${TARGET_DISTRO_DIR}${NC}"
echo -e " * Binary RPMs:        ${BOLD}${RPM_REPO_DIR}${NC}"
echo -e " * Source RPMs:        ${BOLD}${SRPM_REPO_DIR}${NC}"
echo -e " * Repodata Metadata:  ${BOLD}${RPM_REPO_DIR}/repodata/repomd.xml${NC}"
echo -e " * Client Repo Config: ${BOLD}${CLIENT_REPO_FILE}${NC}"
echo -e "==========================================================="
echo -e " Clients can install the repository via:"
echo -e "   sudo curl -Lo /etc/yum.repos.d/${DISTRO_NAME}.repo http://<server-ip>/${DISTRO_NAME}/${DISTRO_NAME}.repo"
echo -e "   sudo dnf --disablerepo='*' --enablerepo='${DISTRO_NAME}' list available"
echo -e "===========================================================\n"
