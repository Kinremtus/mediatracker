# Partial backend configuration for storing this stack's Terraform state in
# Cloudflare R2. R2 speaks the S3 API, so Terraform's built-in `s3` backend
# works and no plugin/extra provider is needed.
#
# Why: the state currently lives only on the workstation that ran `apply`
# (terraform.tfstate, ~121K, contains secrets). Losing it means drift and
# manual `terraform import` of every object. R2 can hold both this state and
# the database dumps (different prefixes) in one bucket.
#
# Enable (once):
#   1. In terraform/monitoring/providers.tf add an EMPTY backend to the
#      existing terraform block:
#
#          terraform {
#            required_version = ">= 1.5"
#            backend "s3" {}
#
#            required_providers { ... }
#          }
#
#      (all real parameters live in this file, so nothing sensitive is
#      committed and the working copy keeps working until you opt in)
#   2. Create the R2 bucket + an API token with Object Read & Write on it.
#   3. Export the token credentials:
#          export AWS_ACCESS_KEY_ID=<token access key id>
#          export AWS_SECRET_ACCESS_KEY=<token secret access key>
#   4. Migrate the local state into R2 (run inside terraform/monitoring/):
#          terraform init -backend-config=backend.r2.example.hcl -migrate-state
#
# After that any `terraform plan/apply` reads and writes state in R2. Keep a
# copy of the bucket name in the README's deployment section.

bucket                      = "mediatracker"
key                         = "monitoring/terraform.tfstate"
region                      = "auto"
endpoints                   = { s3 = "https://<ACCOUNT_ID>.r2.cloudflarestorage.com" }
skip_credentials_validation = true
skip_region_validation      = true
skip_requesting_account_id  = true
skip_metadata_api_check     = true
