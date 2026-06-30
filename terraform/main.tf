resource "cloudflare_record" "main" {
  zone_id = var.cloudflare_zone_id
  name    = "@"
  content = var.server_ip
  type    = "A"
  proxied = true
}