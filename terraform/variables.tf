variable "cloudflare_api_token" {
  description = "Cloudflare API token with DNS edit permissions"
  type        = string
  sensitive   = true
}

variable "cloudflare_zone_id" {
  description = "Cloudflare zone ID for the domain"
  type        = string
}

variable "server_ip" {
  description = "Public IP of InterServer VPS"
  type        = string
}

/* variable "vultr_api_key" {
  description = "VULTR API key"
  type        = string
  sensitive   = true
}

variable "vultr_ssh_key" {
  description = "VULTR SSH public key"
  type        = string
} */