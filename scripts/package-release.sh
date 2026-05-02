#!/usr/bin/env bash
set -euo pipefail

unset CDPATH

if [[ "${LC_ALL:-}" == "C.UTF-8" || "${LANG:-}" == "C.UTF-8" ]]; then
  export LC_ALL="en_US.UTF-8"
  export LANG="en_US.UTF-8"
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && /bin/pwd -P)"
ROOT_DIR="$(git -C "${SCRIPT_DIR}" rev-parse --show-toplevel)"
TARGET_TRIPLE="${1:-}"
VERSION="${2:-}"

if [[ -z "${TARGET_TRIPLE}" || -z "${VERSION}" ]]; then
  echo "Usage: $0 <target-triple> <version>" >&2
  exit 1
fi

BIN_NAME="aag"
DIST_DIR="${ROOT_DIR}/dist"
STAGE_DIR="${DIST_DIR}/${BIN_NAME}-${VERSION}-${TARGET_TRIPLE}"
ARCHIVE_BASENAME="${BIN_NAME}-${VERSION}-${TARGET_TRIPLE}"
ARCHIVE_PATH="${DIST_DIR}/${ARCHIVE_BASENAME}.tar.gz"
TARGET_BIN_PATH="${ROOT_DIR}/target/${TARGET_TRIPLE}/release/${BIN_NAME}"
HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"

mkdir -p "${DIST_DIR}"
rm -rf "${STAGE_DIR}" "${ARCHIVE_PATH}"
mkdir -p "${STAGE_DIR}"

cargo build --release --target "${TARGET_TRIPLE}"

if [[ ! -f "${TARGET_BIN_PATH}" && "${TARGET_TRIPLE}" == "${HOST_TRIPLE}" ]]; then
  TARGET_BIN_PATH="${ROOT_DIR}/target/release/${BIN_NAME}"
fi

if [[ ! -f "${TARGET_BIN_PATH}" ]]; then
  echo "Missing built binary: ${TARGET_BIN_PATH}" >&2
  exit 1
fi

install -m 0755 "${TARGET_BIN_PATH}" "${STAGE_DIR}/${BIN_NAME}"
install -m 0644 "${ROOT_DIR}/README.md" "${STAGE_DIR}/README.md"

tar -C "${DIST_DIR}" -czf "${ARCHIVE_PATH}" "${ARCHIVE_BASENAME}"
shasum -a 256 "${ARCHIVE_PATH}" | awk '{print $1}' > "${ARCHIVE_PATH}.sha256"

echo "Created:"
echo "  ${ARCHIVE_PATH}"
echo "  ${ARCHIVE_PATH}.sha256"
