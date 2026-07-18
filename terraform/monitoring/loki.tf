resource "kubernetes_service_v1" "loki" {
    metadata {
      name      = "loki"
      namespace = "monitoring"
    }
    spec {
      selector = {
        app = "loki-server"
      }
      port {
        port = 3100
        target_port = 3100
      }
    } 
}

resource "kubernetes_config_map_v1" "loki" {
  metadata {
    name = "loki-config"
    namespace = "monitoring"
  }
  data = {
    "loki-config.yaml" = <<EOF
    auth_enabled: false
    server:
      http_listen_port: 3100
    common:
      path_prefix: /loki
    ingester:
      lifecycler:
        ring:
          kvstore:
            store: inmemory
      chunk_idle_period: 15m
      chunk_retain_period: 30s
    schema_config:
      configs:
        - from: 2020-10-24
          store: tsdb
          object_store: filesystem
          schema: v13
          index:
            prefix: index_
            period: 24h
    storage_config:
      tsdb_shipper:
        active_index_directory: /loki/index
        cache_location: /loki/cache
      filesystem:
        directory: /loki/chunks
EOF
  }
}

resource "kubernetes_stateful_set_v1" "loki" {
  metadata {
    name = "loki"
    namespace = "monitoring"
    labels = {
      app = "loki-server"
  }
 }
 spec {
   service_name = "loki"
   replicas = 1
   selector {
     match_labels = {
       app = "loki-server"
     }
   }
   template {
     metadata {
       labels = {
         app = "loki-server"
       }
     }
     spec {
       container {
         name = "loki"
         image = "grafana/loki:3.7.3"
         args = ["-config.file=/etc/loki/loki-config.yaml"]
         port {
           container_port = 3100
         }
         volume_mount {
           name = "config"
           mount_path = "/etc/loki/"
         }
         volume_mount {
           name = "data"
           mount_path = "/loki"
         }
         resources {
           limits = {
             memory = "100Mi"
             cpu = "100m"
           }
         }
       }
       volume {
         name = "config"
         config_map {
           name = "loki-config"
         }
       }
     }
   }
   volume_claim_template {
     metadata {
       name = "data"
     }
     spec {
       access_modes = ["ReadWriteOnce"]
       resources {
         requests = {
           storage = "5Gi"
         }
       }
     }
   }
 }
}