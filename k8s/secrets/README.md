# Encrypted secrets (SOPS + age)

Kubernetes Secrets for this project are stored in git **encrypted** with
[SOPS](https://github.com/getsops/sops) using an [age](https://github.com/FiloSottile/age)
key. The plaintext `.env` stays local and gitignored; only `*.enc.yaml` is committed.

## What lives here

| File | Kubernetes Secret | Keys |
|------|-------------------|------|
| `app-secret.enc.yaml` | `app-secret` | app config + provider API keys + Telegram |
| `db-secret.enc.yaml`  | `db-secret`  | `POSTGRES_USER`, `POSTGRES_PASSWORD`, `POSTGRES_DB` |

The exact key allowlist is defined in `scripts/encrypt-secrets.sh` (it is
explicit on purpose -- we never dump the whole `.env` into the cluster).

## One-time setup

```bash
# 1. create an age key (prints the public key on the first line)
age-keygen -o ~/.config/sops/age/keys.txt
chmod 600 ~/.config/sops/age/keys.txt

# 2. copy that "age1..." public key into .sops.yaml

# 3. back the private key up OFF-SITE. If it is lost, the encrypted files
#    cannot be recovered -- not even by the author.
```

Install `sops` + `age` on whatever machine will encrypt/decrypt. On the VPS the
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

## Rules

- Never commit `.env` or any decrypted `.yaml` (see `.gitignore`).
- The encrypted `*.enc.yaml` files **are** meant to be committed.
- `.sops.yaml` holds only the **public** key and is safe to commit.
