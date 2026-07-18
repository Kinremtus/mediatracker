resource "kubernetes_service_v1" "alertmanager" {
    metadata {
      name      = "alertmanager"
      namespace = "monitoring"
    }
    spec {
      selector = {
        app = "alertmanager-server"
      }
      port {
        port = 9093
        target_port = 9093
      }
    } 
}

resource "kubernetes_config_map_v1" "alertmanager" {
  metadata {
    name = "alertmanager-config"
    namespace = "monitoring"
  }
  data = {
    "alertmanager.yml" = <<EOF
    route:
      receiver: telegram
      repeat_interval: 6h
    receivers:
    - name: telegram
      telegram_configs:
      - bot_token_file: /etc/alertmanager/secrets/token
        chat_id: 796477198
        parse_mode: HTML    
EOF
  }
}

resource "kubernetes_deployment_v1" "alertmanager" {
  metadata {
    name = "alertmanager-deployment"
    namespace = "monitoring"
    labels = {
      app = "alertmanager-server"
    }
  }
  spec {
    replicas = 1
    selector {
      match_labels = {
        app = "alertmanager-server"
      }
    }
    template {
      metadata {
        labels = {
          app = "alertmanager-server"
        }
      }
      spec {
        container {
          name = "alertmanager"
          image = "prom/alertmanager"
          args = ["--config.file=/etc/alertmanager/alertmanager.yml"]
          port {
            container_port = 9093
          }
          volume_mount {
            name = "config"
            mount_path = "/etc/alertmanager/"
          }
          volume_mount {
            name = "telegram-token"
            mount_path = "/etc/alertmanager/secrets/"
            read_only = true
          }
          resources {
            limits = {
              memory = "30Mi"
              cpu = "30m"
            }
          }
        }
        volume {
          name = "config"
          config_map {
            name = "alertmanager-config"
          }
        }
        volume {
          name = "telegram-token"
          secret {
            secret_name = "telegram-bot"
          }
        }
      }
    }
  }
}