resource "cloudflare_record" "main" {
  zone_id = var.cloudflare_zone_id
  name    = "mediatracker"
  type    = "CNAME"
  content = "0f6eebb2-ff0e-49fc-92da-3379ebc12cc1.cfargotunnel.com"
  proxied = true
}