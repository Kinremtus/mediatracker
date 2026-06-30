output "main_domain" {
  description = "Main domain with Cloudflare proxy"
  value       = cloudflare_record.main.hostname
}