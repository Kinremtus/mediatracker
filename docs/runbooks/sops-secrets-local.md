# Runbook: local PC SOPS + age setup

Optional local setup to decrypt/inspect `k8s/secrets/*.enc.yaml` on the
development machine (Arch Linux). Encryption is always done on VPS1 -- the local
`.env` is NOT authoritative.

## 1. Install the tools (Arch Linux)

```bash
sudo pacman -S --needed sops age
# or, with an AUR helper:
# yay -S sops age
sops --version
```

## 2. Copy the private key from VPS1

`ssh` needs `-o ClearAllForwardings=yes` here because of a broken `LocalForward`
in the local SSH config:

```bash
ssh -o ClearAllForwardings=yes VPS1 'cat ~/.config/sops/age/keys.txt' > /tmp/age-keys.txt
mkdir -p ~/.config/sops/age && mv /tmp/age-keys.txt ~/.config/sops/age/keys.txt
chmod 600 ~/.config/sops/age/keys.txt
```

## 3. Verify decryption

```bash
cd ~/mediatracker
sops -d k8s/secrets/app-secret.enc.yaml | head   # must print a Kubernetes Secret YAML
```

## 4. Notes

- The local `.env` is not authoritative. Always encrypt from the VPS1 `.env`;
  the local key is only for debugging/reading.
- On a machine where the plain `kubectl` already sees a kubeconfig,
  `scripts/update-secrets.sh` works as-is.
