#!/usr/bin/env bash
# ==============================================================================
# TacOS Distribution Repository Bulk Promotion and GPG Signing Script
# ==============================================================================
# High-performance, bulk publication recipe for DBS:
#   1. Harvests packages from worker staging directories into dynamic pool
#   2. Promotes binary and source RPMs using BTRFS CoW reflinks (instant, 0 extra disk)
#   3. Intelligently discovers unsigned RPMs and signs in parallel with rpmsign
#   4. Updates repository repodata metadata incrementally via createrepo_c
#   5. Detached-signs repomd.xml with GPG
#   6. Exports public GPG key and generates/updates client .repo configuration
# ==============================================================================

set -euo pipefail

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

# Configuration Defaults
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONFIG_FILE=""
if [[ -f "tacos-distro.toml" ]]; then
    CONFIG_FILE="tacos-distro.toml"
elif [[ -f "tacos.toml" ]]; then
    CONFIG_FILE="tacos.toml"
fi

DISTRO_NAME="tacos-stable-x86_64"
ARCH="x86_64"
STAGING_DIR="/srv/dbs/tacos/staging"
DISTRO_ROOT="/srv/dbs/tacos/distro"
GPG_KEY="release@tacos.org.mx"
BASE_URL="http://repos.tacos.org.mx"
COMPS_FILE=""
WORKERS=$(nproc 2>/dev/null || echo 8)
SIGN_WORKERS=12

# Action Flags
PROMOTE_ONLY=false
SIGN_ONLY=false
SKIP_SRPMS=false
NO_SIGN=false

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

High-throughput bulk promotion and GPG signing recipe for TacOS/DBS.

Options:
  -c, --config <FILE>         Path to DBS configuration TOML [default: ${CONFIG_FILE:-tacos-distro.toml}]
  -n, --name <NAME>           Distribution name [default: ${DISTRO_NAME}]
  -a, --arch <ARCH>           Target architecture [default: ${ARCH}]
  -s, --staging-dir <PATH>    Staging root directory [default: ${STAGING_DIR}]
  -d, --distro-dir <PATH>     Distro repository root [default: ${DISTRO_ROOT}]
  -g, --comps <PATH>          Path to comps.xml package groups file [optional]
  -k, --key <KEY_ID>          GPG key ID/email for signing [default: ${GPG_KEY}]
  -u, --base-url <URL>        Base URL for client .repo [default: ${BASE_URL}]
  -j, --workers <NUM>         Concurrency for createrepo_c [default: ${WORKERS}]
      --sign-workers <NUM>    Concurrency for parallel rpmsign [default: ${SIGN_WORKERS}]
      --promote-only          Only promote packages (skip signing and repodata)
      --sign-only             Only sign existing unsigned packages and repomd.xml
      --no-sign               Skip package and metadata signing
      --skip-srpms            Skip processing source RPMs (.src.rpm)
  -h, --help                  Display this help message

Examples:
  # Bulk promote, sign, index, and publish (standard recipe):
  $(basename "$0")

  # Only promote packages using BTRFS CoW (no signing or indexing):
  $(basename "$0") --promote-only

  # Only sign all unsigned packages in the distro repo:
  $(basename "$0") --sign-only
EOF
    exit 0
}

# 1. First parse --config if specified early
for ((i=1; i<=$#; i++)); do
    case "${!i}" in
        -c|--config)
            next_idx=$((i+1))
            CONFIG_FILE="${!next_idx}"
            ;;
    esac
done

# 2. Load defaults from [distro] section in TOML config if available
if [[ -n "${CONFIG_FILE}" && -f "${CONFIG_FILE}" ]]; then
    log_info "Reading defaults from configuration: ${CONFIG_FILE}"
    eval "$("${SCRIPT_DIR}/get_config.py" "${CONFIG_FILE}")"
fi

# 3. Parse all CLI options to allow overriding configuration file values
while [[ $# -gt 0 ]]; do
    case "$1" in
        -c|--config) CONFIG_FILE="$2"; shift 2 ;;
        -n|--name) DISTRO_NAME="$2"; shift 2 ;;
        -a|--arch) ARCH="$2"; shift 2 ;;
        -s|--staging-dir) STAGING_DIR="$2"; shift 2 ;;
        -d|--distro-dir) DISTRO_ROOT="$2"; shift 2 ;;
        -g|--comps) COMPS_FILE="$2"; shift 2 ;;
        -k|--key) GPG_KEY="$2"; shift 2 ;;
        -u|--base-url) BASE_URL="$2"; shift 2 ;;
        -j|--workers) WORKERS="$2"; shift 2 ;;
        --sign-workers) SIGN_WORKERS="$2"; shift 2 ;;
        --promote-only) PROMOTE_ONLY=true; shift ;;
        --sign-only) SIGN_ONLY=true; shift ;;
        --no-sign) NO_SIGN=true; shift ;;
        --skip-srpms) SKIP_SRPMS=true; shift ;;
        -h|--help) usage ;;
        *) log_error "Unknown option: $1"; usage ;;
    esac
done

TARGET_DISTRO_DIR="${DISTRO_ROOT}/${DISTRO_NAME}"
BINARY_REPO="${TARGET_DISTRO_DIR}/${ARCH}"
SOURCE_REPO="${TARGET_DISTRO_DIR}/source/SRPMS"
STAGING_POOL="${STAGING_DIR}/rpms/${ARCH}"

# Auto-detect comps.xml if not explicitly configured or passed via CLI
if [[ -z "${COMPS_FILE}" ]]; then
    if [[ -f "${TARGET_DISTRO_DIR}/comps.xml" ]]; then
        COMPS_FILE="${TARGET_DISTRO_DIR}/comps.xml"
    elif [[ -f "${DISTRO_ROOT}/${DISTRO_NAME}/comps.xml" ]]; then
        COMPS_FILE="${DISTRO_ROOT}/${DISTRO_NAME}/comps.xml"
    elif [[ -f "${SCRIPT_DIR}/../config/comps.xml" ]]; then
        COMPS_FILE="${SCRIPT_DIR}/../config/comps.xml"
    elif [[ -f "config/comps.xml" ]]; then
        COMPS_FILE="config/comps.xml"
    elif [[ -f "/srv/dbs/tacos/distro/tacos-stable-x86_64/comps.xml" ]]; then
        COMPS_FILE="/srv/dbs/tacos/distro/tacos-stable-x86_64/comps.xml"
    fi
fi

echo "==========================================================="
echo " TacOS Bulk Package Promotion & Signing Recipe"
echo " Distribution:      ${DISTRO_NAME}"
echo " Architecture:      ${ARCH}"
echo " Staging Root:      ${STAGING_DIR}"
echo " Dynamic Pool:      ${STAGING_POOL}"
echo " Binary Repo:       ${BINARY_REPO}"
echo " Source Repo:       ${SOURCE_REPO}"
echo " Comps Metadata:    ${COMPS_FILE:-None (no package groups)}"
echo " GPG Signing Key:   ${GPG_KEY}"
echo " Indexing Workers:  ${WORKERS} threads"
echo " Signing Workers:   ${SIGN_WORKERS} workers"
echo " Mode:              $(if [[ "${PROMOTE_ONLY}" == true ]]; then echo "Promote Only"; elif [[ "${SIGN_ONLY}" == true ]]; then echo "Sign Only"; else echo "Full Bulk Publish"; fi)"
echo "==========================================================="

START_TIME=$(date +%s)

# Ensure target directories exist
mkdir -p "${BINARY_REPO}"
mkdir -p "${SOURCE_REPO}"
mkdir -p "${STAGING_POOL}"

# ------------------------------------------------------------------------------
# Phase 1: Bulk Promotion via BTRFS CoW Reflinks
# ------------------------------------------------------------------------------
if [[ "${SIGN_ONLY}" == false ]]; then
    log_step "Phase 1: Harvesting & Bulk Promoting Packages (BTRFS CoW)"

    # 1. Harvest stranded RPMs from worker directories into staging dynamic pool
    log_info "Scanning worker directories for completed packages..."
    WORKER_HARVEST_COUNT=0
    if [[ -d "${STAGING_DIR}" ]]; then
        find "${STAGING_DIR}" -mindepth 2 -maxdepth 2 -type f -name "*.rpm" ! -path "*/rpms/*" \
            -exec cp -u --reflink=auto -t "${STAGING_POOL}" {} + 2>/dev/null || true
    fi

    # 2. Bulk promote binary RPMs to distro repository
    log_info "Syncing binary RPMs to distro repository: ${BINARY_REPO}"
    find "${STAGING_POOL}" -maxdepth 1 -type f -name "*.rpm" ! -name "*.src.rpm" \
        -exec cp -u --reflink=auto -t "${BINARY_REPO}" {} + 2>/dev/null || true

    # 3. Bulk promote source RPMs to source repository
    if [[ "${SKIP_SRPMS}" == false ]]; then
        log_info "Syncing source RPMs to source repository: ${SOURCE_REPO}"
        find "${STAGING_POOL}" -maxdepth 1 -type f -name "*.src.rpm" \
            -exec cp -u --reflink=auto -t "${SOURCE_REPO}" {} + 2>/dev/null || true
    fi

    BIN_COUNT=$(find "${BINARY_REPO}" -maxdepth 1 -type f -name "*.rpm" | wc -l)
    SRC_COUNT=$(find "${SOURCE_REPO}" -maxdepth 1 -type f -name "*.src.rpm" | wc -l)
    log_success "Promotion complete. Repository contains ${BIN_COUNT} binary RPMs and ${SRC_COUNT} source RPMs."
fi

# ------------------------------------------------------------------------------
# Phase 2: Parallel GPG Signing of Unsigned RPMs
# ------------------------------------------------------------------------------
if [[ "${PROMOTE_ONLY}" == false && "${NO_SIGN}" == false && -n "${GPG_KEY}" ]]; then
    log_step "Phase 2: Bulk GPG Signing Unsigned Packages"

    # Export GPG_TTY if interactive, or suppress tty ioctl warnings
    export GPG_TTY="${GPG_TTY:-$(tty 2>/dev/null || echo "")}"

    # Verify GPG secret key exists
    if ! gpg --list-secret-keys "${GPG_KEY}" >/dev/null 2>&1; then
        log_error "GPG secret key '${GPG_KEY}' not found in local keyring! Skipping signing."
    else
        log_info "Scanning for unsigned RPMs using fast header detection..."

        # Scanner using check_rpm_signature.py to stream null-terminated paths of unsigned RPMs
        scan_unsigned_rpms() {
            local search_dir="$1"
            "${SCRIPT_DIR}/check_rpm_signature.py" -0 --unsigned "${search_dir}"
        }

        # 1. Sign Binary RPMs
        TMP_BIN_LIST=$(mktemp /tmp/dbs_unsigned_bin.XXXXXX)
        scan_unsigned_rpms "${BINARY_REPO}" > "${TMP_BIN_LIST}"
        UNSIGNED_BIN=$(tr -cd '\0' < "${TMP_BIN_LIST}" | wc -c)
        log_info "Found ${UNSIGNED_BIN} unsigned binary package(s) in ${BINARY_REPO}"

        if [[ "${UNSIGNED_BIN}" -gt 0 ]]; then
            log_info "Signing binary packages with ${SIGN_WORKERS} parallel workers..."
            xargs -0 -r -P "${SIGN_WORKERS}" -n 50 \
                rpmsign --define="%_gpg_name ${GPG_KEY}" --addsign < "${TMP_BIN_LIST}" 2>&1 | \
                grep -v -E "No se pudo poner GPG_TTY|Could not set GPG_TTY" || true
            log_success "Binary package signing complete."
        fi
        rm -f "${TMP_BIN_LIST}"

        # 2. Sign Source RPMs
        if [[ "${SKIP_SRPMS}" == false && -d "${SOURCE_REPO}" ]]; then
            TMP_SRC_LIST=$(mktemp /tmp/dbs_unsigned_src.XXXXXX)
            scan_unsigned_rpms "${SOURCE_REPO}" > "${TMP_SRC_LIST}"
            UNSIGNED_SRC=$(tr -cd '\0' < "${TMP_SRC_LIST}" | wc -c)
            log_info "Found ${UNSIGNED_SRC} unsigned source package(s) in ${SOURCE_REPO}"

            if [[ "${UNSIGNED_SRC}" -gt 0 ]]; then
                log_info "Signing source packages with ${SIGN_WORKERS} parallel workers..."
                xargs -0 -r -P "${SIGN_WORKERS}" -n 50 \
                    rpmsign --define="%_gpg_name ${GPG_KEY}" --addsign < "${TMP_SRC_LIST}" 2>&1 | \
                    grep -v -E "No se pudo poner GPG_TTY|Could not set GPG_TTY" || true
                log_success "Source package signing complete."
            fi
            rm -f "${TMP_SRC_LIST}"
        fi
    fi
fi

# ------------------------------------------------------------------------------
# Phase 3: Repository Metadata Indexing (createrepo_c)
# ------------------------------------------------------------------------------
if [[ "${PROMOTE_ONLY}" == false ]]; then
    log_step "Phase 3: Indexing Repository Metadata"

    CREATEREPO_COMPS_ARGS=()
    if [[ -n "${COMPS_FILE}" && -f "${COMPS_FILE}" ]]; then
        log_info "Including package groups from comps file: ${COMPS_FILE}"
        CREATEREPO_COMPS_ARGS=(-g "${COMPS_FILE}")
    elif [[ -n "${COMPS_FILE}" ]]; then
        log_warn "Comps file specified but not found: ${COMPS_FILE} (skipping groups)"
    fi

    log_info "Updating binary repository metadata with ${WORKERS} workers..."
    createrepo_c --update --workers "${WORKERS}" ${CREATEREPO_COMPS_ARGS[@]+"${CREATEREPO_COMPS_ARGS[@]}"} "${BINARY_REPO}"

    if [[ "${SKIP_SRPMS}" == false && -d "${SOURCE_REPO}" ]]; then
        if compgen -G "${SOURCE_REPO}/*.src.rpm" > /dev/null; then
            log_info "Updating source repository metadata with ${WORKERS} workers..."
            createrepo_c --update --workers "${WORKERS}" "${SOURCE_REPO}"
        fi
    fi
    log_success "Metadata indexing complete."
fi

# ------------------------------------------------------------------------------
# Phase 4: Repodata GPG Signing
# ------------------------------------------------------------------------------
if [[ "${PROMOTE_ONLY}" == false && "${NO_SIGN}" == false && -n "${GPG_KEY}" ]]; then
    log_step "Phase 4: Detached Signing repomd.xml Metadata"

    BIN_REPOMD="${BINARY_REPO}/repodata/repomd.xml"
    if [[ -f "${BIN_REPOMD}" ]]; then
        log_info "Signing binary repomd.xml..."
        gpg --detach-sign --armor --batch --yes -u "${GPG_KEY}" "${BIN_REPOMD}"
        log_success "Binary repodata signed: ${BIN_REPOMD}.asc"
    fi

    SRC_REPOMD="${SOURCE_REPO}/repodata/repomd.xml"
    if [[ -f "${SRC_REPOMD}" ]]; then
        log_info "Signing source repomd.xml..."
        gpg --detach-sign --armor --batch --yes -u "${GPG_KEY}" "${SRC_REPOMD}"
        log_success "Source repodata signed: ${SRC_REPOMD}.asc"
    fi
fi

# ------------------------------------------------------------------------------
# Phase 5: Export GPG Key & Generate Client Configuration
# ------------------------------------------------------------------------------
if [[ "${PROMOTE_ONLY}" == false ]]; then
    log_step "Phase 5: Generating Client Repository Configuration"

    # Export Public Key
    KEY_FILE="${TARGET_DISTRO_DIR}/RPM-GPG-KEY-${DISTRO_NAME}"
    if [[ -n "${GPG_KEY}" ]]; then
        log_info "Exporting GPG public key to ${KEY_FILE}..."
        gpg --armor --export "${GPG_KEY}" > "${KEY_FILE}" 2>/dev/null || true
    fi

    # Write Client .repo configuration
    CLIENT_REPO="${TARGET_DISTRO_DIR}/${DISTRO_NAME}.repo"
    log_info "Writing client repository file to ${CLIENT_REPO}..."
    cat <<EOF > "${CLIENT_REPO}"
# TacOS Distribution Repository Configuration
# Generated by DBS bulk publisher

[${DISTRO_NAME}]
name=TacOS Linux - ${DISTRO_NAME} (\$basearch)
baseurl=${BASE_URL%/}/${DISTRO_NAME}/\$basearch
enabled=1
gpgcheck=$(if [[ "${NO_SIGN}" == false && -n "${GPG_KEY}" ]]; then echo "1"; else echo "0"; fi)
repo_gpgcheck=$(if [[ "${NO_SIGN}" == false && -n "${GPG_KEY}" ]]; then echo "1"; else echo "0"; fi)
$(if [[ "${NO_SIGN}" == false && -n "${GPG_KEY}" ]]; then echo "gpgkey=${BASE_URL%/}/${DISTRO_NAME}/RPM-GPG-KEY-${DISTRO_NAME}"; fi)
metadata_expire=300
skip_if_unavailable=False

[${DISTRO_NAME}-source]
name=TacOS Linux - ${DISTRO_NAME} (Source)
baseurl=${BASE_URL%/}/${DISTRO_NAME}/source/SRPMS
enabled=0
gpgcheck=$(if [[ "${NO_SIGN}" == false && -n "${GPG_KEY}" ]]; then echo "1"; else echo "0"; fi)
repo_gpgcheck=$(if [[ "${NO_SIGN}" == false && -n "${GPG_KEY}" ]]; then echo "1"; else echo "0"; fi)
$(if [[ "${NO_SIGN}" == false && -n "${GPG_KEY}" ]]; then echo "gpgkey=${BASE_URL%/}/${DISTRO_NAME}/RPM-GPG-KEY-${DISTRO_NAME}"; fi)
metadata_expire=300
skip_if_unavailable=False
EOF
    log_success "Client repo written: ${CLIENT_REPO}"
fi

# ------------------------------------------------------------------------------
# Summary & Timing Report
# ------------------------------------------------------------------------------
END_TIME=$(date +%s)
TOTAL_TIME=$((END_TIME - START_TIME))

TOTAL_BIN=$(find "${BINARY_REPO}" -maxdepth 1 -type f -name "*.rpm" 2>/dev/null | wc -l)
TOTAL_SRC=$(find "${SOURCE_REPO}" -maxdepth 1 -type f -name "*.src.rpm" 2>/dev/null | wc -l)

echo ""
echo "==========================================================="
echo -e "${GREEN}${BOLD}✓ TacOS Bulk Distribution Promotion & Signing Finished!${NC}"
echo "==========================================================="
echo " Total Binary RPMs in Distro: ${TOTAL_BIN}"
echo " Total Source RPMs in Distro: ${TOTAL_SRC}"
echo " Repodata Location:           ${BINARY_REPO}/repodata/"
echo " Client .repo File:           ${TARGET_DISTRO_DIR}/${DISTRO_NAME}.repo"
echo " GPG Signed:                  $(if [[ "${NO_SIGN}" == false && -n "${GPG_KEY}" ]]; then echo "Yes (${GPG_KEY})"; else echo "No"; fi)"
echo " Elapsed Time:                ${TOTAL_TIME}s ($((TOTAL_TIME / 60))m $((TOTAL_TIME % 60))s)"
echo "==========================================================="
