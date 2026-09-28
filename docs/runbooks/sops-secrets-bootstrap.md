# Runbook: bootstrap SOPS + age secrets on VPS1

One-time operational runbook that activates the SOPS/age workflow so that
`k8s/secrets/*.enc.yaml` can be decrypted by CI before `helm upgrade`. Run it
MANUALLY on VPS1, in order. The agent/CI cannot do this: it needs the real
private key and the real secret values.

> The private age key is the ONLY way to decrypt `*.enc.yaml`. Losing it means
> losing the secrets forever. The off-site backup (section 4) is mandatory.

## 1. Preconditions

- VPS1: Debian 12, x86_64, passwordless `sudo`.
- The repository is checked out at `~/mediatracker` (or `TARGET_DIR` from the
  GitHub Secrets).
- The deploy user (the one CI logs in as over SSH) is the user running this
  runbook. CI reads `$HOME/.config/sops/age/keys.txt` for that user.
- The live `app-secret` exists in namespace `mediatracker`.

## 2. Install the tools

```bash
sudo apt-get update && sudo apt-get install -y age
curl -fsSL -o /tmp/sops https://github.com/getsops/sops/releases/download/v3.13.3/sops-v3.13.3.linux.amd64
sudo install -m 0755 /tmp/sops /usr/local/bin/sops && rm -f /tmp/sops
age --version && sops --version   # expect sops 3.13.3
```

## 3. Generate the age key (NO sudo -- as the deploy user)

```bash
mkdir -p ~/.config/sops/age
age-keygen -o ~/.config/sops/age/keys.txt
chmod 600 ~/.config/sops/age/keys.txt
age-keygen -y ~/.config/sops/age/keys.txt   # prints the public age1... key
```

## 4. Back the private key up IMMEDIATELY (copy to the local PC)

The private key is the ONLY way to decrypt `*.enc.yaml`. If it is lost, the
secrets are lost forever. The backup host is the local PC (Arch Linux); the
same copy doubles as the local decryption key -- full setup in
`sops-secrets-local.md`.

Run this ON THE LOCAL PC right after step 3:

```bash
mkdir -p ~/.config/sops/age
scp -o ClearAllForwardings=yes VPS1:~/.config/sops/age/keys.txt ~/.config/sops/age/keys.txt
chmod 600 ~/.config/sops/age/keys.txt
age-keygen -y ~/.config/sops/age/keys.txt   # must print the same age1... as step 3
```

Rules:

- The key file is plaintext. Never commit it, never put it in an unencrypted
  cloud-synced folder, keep `chmod 600` on both hosts.
- Consider a third copy in a password manager: with only VPS1 + PC, losing both
  machines loses the secrets.

## 5. Put the public key into `.sops.yaml`

Replace `age1REPLACE_WITH_YOUR_AGE_PUBLIC_KEY` in `.sops.yaml` with the real
`age1...` value printed above, then check the diff:

```bash
cd ~/mediatracker
git diff .sops.yaml
```

## 6. Backfill missing keys into `.env` from the live Secret

Otherwise these values are lost during encryption:

```bash
cd ~/mediatracker
sudo kubectl get secret app-secret -n mediatracker -o jsonpath='{.data.COMIC_VINE_API_KEY}' | base64 -d; echo
sudo kubectl get secret app-secret -n mediatracker -o jsonpath='{.data.HARDCOVER_API_KEY}'  | base64 -d; echo
# add both lines to ~/mediatracker/.env:
#   COMIC_VINE_API_KEY=<value>
#   HARDCOVER_API_KEY=<value>
grep -c '^COMIC_VINE_API_KEY=' .env   # must be 1
grep -c '^HARDCOVER_API_KEY='  .env   # must be 1
```

## 7. Generate the encrypted files

```bash
cd ~/mediatracker
SOPS_AGE_KEY_FILE=~/.config/sops/age/keys.txt scripts/encrypt-secrets.sh
grep -c 'ENC\[' k8s/secrets/app-secret.enc.yaml   # > 0
grep -c 'ENC\[' k8s/secrets/db-secret.enc.yaml    # > 0
```

Confirm no plaintext values leaked into the encrypted files.

## 8. Commit and push

```bash
git add .sops.yaml k8s/secrets/*.enc.yaml
git commit -m "chore(secrets): add SOPS-encrypted app/db secrets"
git push
```

Do NOT commit `thoughts/`.

## 9. Rollback

If something is wrong:

```bash
git checkout -- k8s/secrets/ .sops.yaml
```

The live cluster is not touched until CI applies the new secrets.
