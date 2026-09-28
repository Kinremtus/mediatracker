# Runbook: local PC SOPS + age setup (private key backup)

The local PC (Arch Linux) is the **backup host for the age private key** and can
decrypt/inspect `k8s/secrets/*.enc.yaml` locally. It is NOT the encryption
source: encryption always runs on VPS1 from its `.env`.

## 1. Install the tools (Arch Linux)

```bash
sudo pacman -S --needed sops age
# or, with an AUR helper:
# yay -S sops age
sops --version
```

## 2. Copy the private key from VPS1 (this IS the backup) (this IS the backup)

`ssh`/`scp` need `-o ClearAllForwardings=yes` here because of a broken
`LocalForward` in the local SSH config:

```bash
mkdir -p ~/.config/sops/age
scp -o ClearAllForwardings=yes VPS1:~/.config/sops/age/keys.txt ~/.config/sops/age/keys.txt
chmod 600 ~/.config/sops/age/keys.txt
age-keygen -y ~/.config/sops/age/keys.txt   # must match the age1... in .sops.yaml
```

## 3. Verify decryption

```bash
cd ~/mediatracker
sops -d k8s/secrets/app-secret.enc.yaml | head   # must print a Kubernetes Secret YAML
```

## 4. Notes

- The local `.env` is not authoritative. Always encrypt from the VPS1 `.env`;
  the local key is only for debugging/reading.
- The key file is plaintext: never commit it, never put it in an unencrypted
  cloud-synced folder, keep `chmod 600` on both hosts.
- Consider a third copy in a password manager -- losing both VPS1 and the PC
  means losing the secrets.
- On a machine where the plain `kubectl` already sees a kubeconfig,
  `scripts/update-secrets.sh` works as-is.
