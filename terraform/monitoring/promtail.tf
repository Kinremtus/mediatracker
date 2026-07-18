resource "kubernetes_config_map_v1" "promtail" {
  metadata {
    name = "promtail-config"
    namespace = "monitoring"
  }
  data = {
    "promtail-config.yaml" = <<EOF
    server:
      http_listen_port: 9080
    clients:
      - url: http://loki:3100/loki/api/v1/push
    positions:
      filename: /tmp/positions.yaml
    scrape_configs:
      - job_name: kubernetes-pods
        pipeline_stages:
          - cri: {}
        kubernetes_sd_configs:
          - role: pod
        relabel_configs:
          - source_labels: [__meta_kubernetes_namespace]
            target_label: namespace
          - source_labels: [__meta_kubernetes_pod_name]
            target_label: pod
          - source_labels: [__meta_kubernetes_pod_label_app]
            target_label: app
          - source_labels: [__meta_kubernetes_pod_container_name]
            target_label: container   
EOF
  }
}

resource "kubernetes_daemon_set_v1" "promtail" {
  metadata {
    name = "promtail-deployment"
    namespace = "monitoring"
    labels = {
      app = "promtail-server"
    }
  }
  spec {
    selector {
      match_labels = {
        app = "promtail-server"
      }
    }
    template {
      metadata {
        labels = {
          app = "promtail-server"
        }
      }
      spec {
        container {
          name = "promtail"
          image = "grafana/promtail"
          args = ["-config.file=/etc/promtail/promtail-config.yaml"]
          volume_mount {
            name = "config"
            mount_path = "/etc/promtail/"
          }
          volume_mount {
            name = "varlog"
            mount_path = "/var/log"
          }
          volume_mount {
            name = "dockercontainers"
            mount_path = "/var/lib/docker/containers"
            read_only = true
          }
          resources {
            limits = {
              memory = "30Mi"
              cpu = "20m"
            }
          }
        }
        volume {
          name = "config"
          config_map {
            name = "promtail-config"
          }
        }
        volume {
          name = "varlog"
          host_path {
            path = "/var/log"
          }
        }
        volume {
          name = "dockercontainers"
          host_path {
            path = "/var/lib/docker/containers"
          }
        }
      }
    }
  }
}