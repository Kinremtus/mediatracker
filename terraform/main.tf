resource "cloudflare_record" "main" {
  zone_id = var.cloudflare_zone_id
  name    = "mediatracker"
  type    = "CNAME"
  content = "0f6eebb2-ff0e-49fc-92da-3379ebc12cc1.cfargotunnel.com"
  proxied = true
}
/* VULTR resources — commented out. Uncomment and run `terraform apply` to recreate.
resource "vultr_ssh_key" "main" {
  name    = "mediatracker-key"
  ssh_key = var.vultr_ssh_key
}

resource "vultr_instance" "media" {
  plan        = "vc2-1c-1gb"
  region      = "ams"        # Amsterdam
  os_id       = 2136         # Ubuntu 24.04 x64
  hostname    = "vultr-mediatracker"
  label       = "MediaTracker VULTR"
  ssh_key_ids = [vultr_ssh_key.main.id]
  backups = "disabled"

  user_data = <<-EOF
    #!/bin/bash
    apt-get update
    apt-get install -y docker.io docker-compose-v2
    systemctl enable docker
    systemctl start docker
  EOF
}

resource "cloudflare_record" "dev" {
  zone_id = var.cloudflare_zone_id
  name    = "dev"
  type    = "A"
  content = vultr_instance.media.main_ip
  proxied = true
}
*/
