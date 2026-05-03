#!/usr/bin/env bash
set -euo pipefail

unset CDPATH
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && /bin/pwd -P)"
ROOT_DIR="$(git -C "${SCRIPT_DIR}" rev-parse --show-toplevel)"
VERSION="${1:-}"
OWNER="${2:-cloudinstone}"
REPO="${3:-aag-cli}"

if [[ -z "${VERSION}" ]]; then
  echo "Usage: $0 <version> [owner] [repo]" >&2
  exit 1
fi

ARM_SHA_FILE="${ROOT_DIR}/dist/aag-${VERSION}-aarch64-apple-darwin.tar.gz.sha256"
INTEL_SHA_FILE="${ROOT_DIR}/dist/aag-${VERSION}-x86_64-apple-darwin.tar.gz.sha256"

if [[ ! -f "${ARM_SHA_FILE}" || ! -f "${INTEL_SHA_FILE}" ]]; then
  echo "Missing checksum files in dist/. Run package-release.sh for both macOS targets first." >&2
  exit 1
fi

ARM_SHA="$(cat "${ARM_SHA_FILE}")"
INTEL_SHA="$(cat "${INTEL_SHA_FILE}")"

sed \
  -e "s|__VERSION__|${VERSION}|g" \
  -e "s|__OWNER__|${OWNER}|g" \
  -e "s|__REPO__|${REPO}|g" \
  -e "s|__ARM_SHA__|${ARM_SHA}|g" \
  -e "s|__INTEL_SHA__|${INTEL_SHA}|g" \
  "${ROOT_DIR}/packaging/homebrew/aag.rb.template" > "${ROOT_DIR}/dist/aag.rb"

echo "Created:"
echo "  ${ROOT_DIR}/dist/aag.rb"
