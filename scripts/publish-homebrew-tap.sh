#!/usr/bin/env bash
set -euo pipefail

unset CDPATH
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && /bin/pwd -P)"
ROOT_DIR="$(git -C "${SCRIPT_DIR}" rev-parse --show-toplevel)"
TAP_REPO="${HOMEBREW_TAP_REPO:-}"
TAP_TOKEN="${HOMEBREW_TAP_TOKEN:-}"
FORMULA_NAME="${FORMULA_NAME:-aag}"
TMP_DIR="$(mktemp -d)"

cleanup() {
  rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

if [[ -z "${TAP_REPO}" || -z "${TAP_TOKEN}" ]]; then
  echo "Homebrew tap secrets are not configured, skipping tap publish"
  exit 0
fi

FORMULA_SOURCE="${ROOT_DIR}/dist/${FORMULA_NAME}.rb"
if [[ ! -f "${FORMULA_SOURCE}" ]]; then
  echo "Missing formula file: ${FORMULA_SOURCE}" >&2
  exit 1
fi

git clone "https://x-access-token:${TAP_TOKEN}@github.com/${TAP_REPO}.git" "${TMP_DIR}/tap" >/dev/null 2>&1
mkdir -p "${TMP_DIR}/tap/Formula"
install -m 0644 "${FORMULA_SOURCE}" "${TMP_DIR}/tap/Formula/${FORMULA_NAME}.rb"

if git -C "${TMP_DIR}/tap" diff --quiet -- Formula/"${FORMULA_NAME}.rb"; then
  echo "No tap changes to publish"
  exit 0
fi

git -C "${TMP_DIR}/tap" add Formula/"${FORMULA_NAME}.rb"
git -C "${TMP_DIR}/tap" \
  -c user.name="${GIT_AUTHOR_NAME:-cloudinstone-bot}" \
  -c user.email="${GIT_AUTHOR_EMAIL:-cloudinstone-bot@users.noreply.github.com}" \
  commit -m "Update ${FORMULA_NAME} formula" >/dev/null
git -C "${TMP_DIR}/tap" push origin HEAD >/dev/null

echo "Updated tap repository: ${TAP_REPO}"
