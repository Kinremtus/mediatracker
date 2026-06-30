output "main_domain" {
  description = "Main domain with Cloudflare proxy"
  value       = cloudflare_record.main.hostname
}

/* output "vultr_ip" {
  description = "VULTR VPS public IP"
  value       = vultr_instance.media.main_ip
} */