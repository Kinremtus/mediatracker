#!/usr/bin/env bash
# Encrypt the application + database secrets with SOPS/age and store them in git.
#
# Usage:
#   scripts/encrypt-secrets.sh [path-to-.env]
#
# Requires: sops, age, kubectl. The plaintext .env is only READ; the temp files
# that hold the generated Secret YAML are shredded on exit, so plaintext never
# lands in the repository.
set -euo pipefail

ENV_FILE="${1:-$HOME/mediatracker/.env}"
SECRETS_DIR="k8s/secrets"
NAMESPACE="mediatracker"

# Explicit allowlist: only these keys ever reach the cluster Secret. Anything
# else in .env (editor vars, unrelated tokens) is intentionally ignored.
APP_KEYS=(
  APP_BASE_URL EMAIL_FROM HOST PORT RUST_LOG SECRET_KEY DATABASE_URL
  TMDB_API_KEY RAWG_API_KEY COMIC_VINE_API_KEY
  IGDB_CLIENT_ID IGDB_CLIENT_SECRET
  MAL_CLIENT_ID MAL_CLIENT_SECRET
  RESEND_API_KEY MAILERSEND_SMTP_USER MAILERSEND_SMTP_PASS
  TELEGRAM_BOT_TOKEN TELEGRAM_CHAT_ID
)
DB_KEYS=(POSTGRES_USER POSTGRES_PASSWORD POSTGRES_DB)

for tool in sops age kubectl; do
  command -v "$tool" >/dev/null 2>&1 || { echo "[!] missing tool: $tool" >&2; exit 1; }
done
[ -f "$ENV_FILE" ] || { echo "[!] env file not found: $ENV_FILE" >&2; exit 1; }
[ -f .sops.yaml ] || { echo "[!] .sops.yaml missing (set your age public key)" >&2; exit 1; }

mkdir -p "$SECRETS_DIR"

filter_env() {
  local key line
  for key in "$@"; do
    line="$(grep -E "^${key}=" "$ENV_FILE" | tail -n1 || true)"
    if [ -n "$line" ]; then
      printf '%s\n' "$line"
    else
      # Non-fatal: surface missing optional keys but keep going.
      echo "[?] key not found in .env: $key" >&2
    fi
  done
}

encrypt_secret() {
  local name="$1"; shift
  local tmp_env tmp_yaml out
  tmp_env="$(mktemp)"
  tmp_yaml="$(mktemp)"
  out="$SECRETS_DIR/${name}.enc.yaml"

  filter_env "$@" > "$tmp_env"
  kubectl create secret generic "$name" -n "$NAMESPACE" \
    --from-env-file="$tmp_env" --dry-run=client -o yaml > "$tmp_yaml"
  sops --encrypt --filename-override "$out" "$tmp_yaml" > "$out"

  shred -u "$tmp_env" "$tmp_yaml" 2>/dev/null || rm -f "$tmp_env" "$tmp_yaml"
  echo "[OK] wrote $out"
}

encrypt_secret app-secret "${APP_KEYS[@]}"
encrypt_secret db-secret  "${DB_KEYS[@]}"

echo "[OK] encrypted secrets ready to commit"
