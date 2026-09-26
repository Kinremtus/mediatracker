resource "kubernetes_persistent_volume_claim_v1" "uptime-kuma" {
  wait_until_bound = false

  metadata {
    name      = "uptime-kuma-data"
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

resource "kubernetes_service_v1" "uptime-kuma" {
  metadata {
    name      = "uptime-kuma"
    namespace = "monitoring"
  }
  spec {
    type = "NodePort"

    selector = {
      app = "uptime-kuma-server"
    }
    port {
      port        = 3001
      target_port = 3001
      node_port   = 30011
    }
  }

}

resource "kubernetes_deployment_v1" "uptime-kuma" {
  metadata {
    name      = "uptime-kuma-deployment"
    namespace = "monitoring"
    labels = {
      app = "uptime-kuma-server"
    }
  }
  spec {
    replicas = 1

    selector {
      match_labels = {
        app = "uptime-kuma-server"
      }
    }

    template {
      metadata {
        labels = {
          app = "uptime-kuma-server"
        }
      }
      spec {
        container {
          name  = "uptime-kuma"
          image = "louislam/uptime-kuma:1"
          port {
            container_port = 3001

          }
          volume_mount {
            name       = "data"
            mount_path = "/app/data"
          }
          resources {
            requests = {
              memory = "128Mi"
              cpu    = "50m"
            }
            limits = {
              memory = "128Mi"
              cpu    = "100m"
            }
          }
        }
        enable_service_links = false

        volume {
          name = "data"
          persistent_volume_claim {
            claim_name = "uptime-kuma-data"
          }
        }
      }
    }
  }
}