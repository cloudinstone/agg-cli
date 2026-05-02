#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
ROOT_DIR="$(CDPATH= cd -- "${SCRIPT_DIR}/.." && pwd -P)"
TMP_DIR="$(mktemp -d)"
TARGET_REPO="${TARGET_REPO:-}"
TARGET_BRANCH="${TARGET_BRANCH:-main}"
COMMIT_MESSAGE="${COMMIT_MESSAGE:-Sync public snapshot}"
TARGET_REMOTE_NAME="${TARGET_REMOTE_NAME:-public}"
KEEP_TMP="${KEEP_TMP:-0}"
RSYNC_BIN="${RSYNC_BIN:-rsync}"

cleanup() {
  if [[ "${KEEP_TMP}" != "1" ]]; then
    rm -rf "${TMP_DIR}"
  else
    printf 'Temporary export kept at %s\n' "${TMP_DIR}"
  fi
}
trap cleanup EXIT

if ! command -v "${RSYNC_BIN}" >/dev/null 2>&1; then
  echo "rsync is required" >&2
  exit 1
fi

if [[ -z "${TARGET_REPO}" ]] && git -C "${ROOT_DIR}" rev-parse --git-dir >/dev/null 2>&1; then
  TARGET_REPO="$(git -C "${ROOT_DIR}" remote get-url "${TARGET_REMOTE_NAME}" 2>/dev/null || true)"
fi

if [[ -z "${TARGET_REPO}" ]]; then
  echo "TARGET_REPO is not set and remote '${TARGET_REMOTE_NAME}' is unavailable" >&2
  exit 1
fi

mkdir -p "${TMP_DIR}/repo"

"${RSYNC_BIN}" -a --delete \
  --exclude '.git' \
  --exclude '.github' \
  --exclude 'target' \
  --exclude 'internal' \
  --exclude '.DS_Store' \
  "${ROOT_DIR}/" "${TMP_DIR}/repo/"

git -C "${TMP_DIR}/repo" init -b "${TARGET_BRANCH}" >/dev/null
git -C "${TMP_DIR}/repo" add .

if git -C "${TMP_DIR}/repo" diff --cached --quiet; then
  echo "Nothing to publish"
  exit 0
fi

git -C "${TMP_DIR}/repo" \
  -c user.name="${GIT_AUTHOR_NAME:-cloudinstone-bot}" \
  -c user.email="${GIT_AUTHOR_EMAIL:-cloudinstone-bot@users.noreply.github.com}" \
  commit -m "${COMMIT_MESSAGE}" >/dev/null

git -C "${TMP_DIR}/repo" remote add origin "${TARGET_REPO}"
git -C "${TMP_DIR}/repo" push --force origin "${TARGET_BRANCH}"
