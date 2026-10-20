terraform {
  required_version = ">= 1.5"

  # State lives in Cloudflare R2 (S3 API). All parameters come from
  # backend.r2.example.hcl via `terraform init -backend-config=...` — nothing
  # sensitive is committed, and the local state is migrated once with
  # `-migrate-state` (see that file for the exact steps).
  backend "s3" {}

  required_providers {
    kubernetes = {
      source  = "hashicorp/kubernetes"
      version = "~> 2.0"
    }
  }
}

provider "kubernetes" {
  config_path = "~/.kube/config-vps"
  insecure    = true
}