resource "kubernetes_persistent_volume_claim_v1" "grafana" {
  wait_until_bound = false

  metadata {
    name      = "grafana-data"
    namespace = "monitoring"
  }
  spec {
    access_modes = ["ReadWriteOnce"]
    resources {
      requests = {
        storage = "1Gi"
      }
    }
  }
}

resource "kubernetes_service_v1" "grafana" {
    metadata {
      name      = "grafana"
      namespace = "monitoring"
    }
    spec {
      type = "NodePort"

      selector = {
        app = "grafana-server"
      }
      port {
        port = 3000
        target_port = 3000
        node_port   = 30001
      }
    }
  
}

resource "kubernetes_config_map_v1" "grafana" {
  metadata {
    name = "grafana-datasource-config"
    namespace = "monitoring"
  }

  data = {
    "datasources.yaml" = <<EOF
    apiVersion: 1
    datasources:
      - name: Prometheus
        type: prometheus
        url: http://prometheus.monitoring:9090
        access: proxy
        isDefault: true
      - name: Loki
        type: loki
        url: http://loki.monitoring:3100
        access: proxy
EOF
  }
}

resource "kubernetes_deployment_v1" "grafana" {
    metadata {
      name = "grafana-deployment"
      namespace = "monitoring"
      labels = {
        app = "grafana-server"
      }
    }
    spec {
      replicas = 1
      selector {
        match_labels = {
          app = "grafana-server"
        }
      }
      template {
        metadata {
          labels = {
            app = "grafana-server"
          }
        }
        spec {
          container {
            name = "grafana"
            image = "grafana/grafana"
            port {
              container_port = 3000
            }
            volume_mount {
              name = "data"
              mount_path = "/var/lib/grafana"
            }
            volume_mount {
              name = "datasources"
              mount_path = "/etc/grafana/provisioning/datasources/"
            }
            resources {
              requests = {
                memory = "256Mi"
                cpu = "100m"
              }
              limits = {
                memory = "512Mi"
                cpu = "200m"
              }
            }
          }
          volume {
            name = "data"
            persistent_volume_claim {
              claim_name = "grafana-data"
            }
          }
          volume {
            name = "datasources"
            config_map {
              name = "grafana-datasource-config"
            }
          }
        }
      }
    }
}