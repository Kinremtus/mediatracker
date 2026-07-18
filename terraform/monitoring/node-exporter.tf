resource "kubernetes_service_v1" "node-exporter" {
    metadata {
      name      = "node-exporter"
      namespace = "monitoring"
    }
    spec {
      selector = {
        app = "node-exporter-server"
      }
      port {
        port = 9100
        target_port = 9100
      }
    } 
}

resource "kubernetes_daemon_set_v1" "node-exporter" {
  metadata {
    name = "node-exporter-deployment"
    namespace = "monitoring"
    labels = {
      app = "node-exporter-server"
    }
  }
  spec {
    selector {
      match_labels = {
        app = "node-exporter-server"
      }
    }
    template {
      metadata {
        labels = {
          app = "node-exporter-server"
        }
      }
      spec {
        host_network = true
        host_pid = true
        container {
          name = "node-exporter"
          image = "prom/node-exporter"
          port {
            container_port = 9100
          }
          volume_mount {
            name = "proc"
            mount_path = "/host/proc"
            read_only = true
          }
          volume_mount {
            name = "sys"
            mount_path = "/host/sys"
            read_only = true
          }
          volume_mount {
            name = "rootfs"
            mount_path = "/host/root"
            read_only = true
          }
          resources {
            limits = {
              memory = "20Mi"
              cpu = "20m"
            }
          }
        }
        volume {
          name = "proc"
          host_path {
            path = "/proc"
          }
        }
        volume {
          name = "sys"
          host_path {
            path = "/sys"
          }
        }
        volume {
          name = "rootfs"
          host_path {
            path = "/"
          }
        }
      }
    }
  }
}