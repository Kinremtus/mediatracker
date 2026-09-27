# Encrypted secrets (SOPS + age)

Kubernetes Secrets for this project are stored in git **encrypted** with
[SOPS](https://github.com/getsops/sops) using an [age](https://github.com/FiloSottile/age)
key. The plaintext `.env` stays local and gitignored; only `*.enc.yaml` is committed.

## What lives here

| File | Kubernetes Secret | Keys |
|------|-------------------|------|
| `app-secret.enc.yaml` | `app-secret` | `SECRET_KEY`, `DATABASE_URL`, provider API keys, Telegram, mail keys |
| `db-secret.enc.yaml`  | `db-secret`  | `POSTGRES_USER`, `POSTGRES_PASSWORD`, `POSTGRES_DB` |

The exact key allowlist is defined in `scripts/encrypt-secrets.sh` (it is
explicit on purpose -- we never dump the whole `.env` into the cluster).

Config-only keys (`HOST`, `PORT`, `RUST_LOG`, `APP_BASE_URL`, `EMAIL_FROM`) are
**deliberately excluded** from `app-secret`: they belong to the ConfigMap
`app-config`, which is the authoritative source for configuration. Keeping them
out of the Secret avoids two sources of truth.

## Install sops + age

Debian 12 (VPS1):

```bash
sudo apt-get update && sudo apt-get install -y age
curl -fsSL -o /tmp/sops https://github.com/getsops/sops/releases/download/v3.13.3/sops-v3.13.3.linux.amd64
sudo install -m 0755 /tmp/sops /usr/local/bin/sops
rm -f /tmp/sops
sops --version   # expect 3.13.3
```

Arch Linux (local PC):

```bash
sudo pacman -S --needed sops age
sops --version
```

## One-time setup

```bash
# 1. create an age key (prints the public key on the first line)
age-keygen -o ~/.config/sops/age/keys.txt
chmod 600 ~/.config/sops/age/keys.txt

# 2. copy that "age1..." public key into .sops.yaml
#    (replace the age1REPLACE_WITH_YOUR_AGE_PUBLIC_KEY placeholder)

# 3. back the private key up OFF-SITE. If it is lost, the encrypted files
#    cannot be recovered -- not even by the author.
```

The key must belong to the **deploy user** -- the user whose `$HOME` the CI
deploy step uses when it reads `$HOME/.config/sops/age/keys.txt`. On the VPS the
private key must exist at `~/.config/sops/age/keys.txt` (override with
`SOPS_AGE_KEY_FILE`).

## Encrypt / apply / rotate

```bash
# encrypt (reads ~/mediatracker/.env, writes k8s/secrets/*.enc.yaml)
scripts/encrypt-secrets.sh

# apply to the cluster (decrypts in a pipe, plaintext never hits disk)
scripts/update-secrets.sh

# rotate a leaked key: edit .env, re-run encrypt-secrets.sh, then update-secrets.sh
```

### Rotation

1. Edit `.env` (source of truth on VPS1) with the new value.
2. Re-run `scripts/encrypt-secrets.sh` -- committing the regenerated
   `*.enc.yaml` updates the cluster on the next deploy.
3. Either `git commit && git push` (`k8s/secrets/*.enc.yaml`) or apply now with
   `scripts/update-secrets.sh`.
4. Re-running encryption is idempotent: the allowlist and namespace do not
   change, only encrypted values.

## Bootstrap / activation runbook

- [`docs/runbooks/sops-secrets-bootstrap.md`](../../docs/runbooks/sops-secrets-bootstrap.md) --
  one-time activation on VPS1 (install tools, generate key, encrypt, commit).
- [`docs/runbooks/sops-secrets-local.md`](../../docs/runbooks/sops-secrets-local.md) --
  optional local PC setup (decrypt/inspect).

## Rules

- Never commit `.env` or any decrypted `.yaml` (see `.gitignore`).
- The encrypted `*.enc.yaml` files **are** meant to be committed.
- `.sops.yaml` holds only the **public** key and is safe to commit.
- The `mediatracker` namespace must exist before secrets are applied; the CI
  deploy step creates it (`kubectl create namespace mediatracker`) if missing.
