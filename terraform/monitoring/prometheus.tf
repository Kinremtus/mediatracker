resource "kubernetes_persistent_volume_claim_v1" "prometheus" {
  wait_until_bound = false

  metadata {
    name      = "prometheus-data"
    namespace = "monitoring"
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

resource "kubernetes_service_v1" "prometheus" {
    metadata {
      name      = "prometheus"
      namespace = "monitoring"
    }
    spec {
      type = "NodePort"
      
      selector = {
        app = "prometheus-server"
      }
      port {
        port = 9090
        target_port = 9090
        node_port   = 30909
      }
    }
  
}

resource "kubernetes_config_map_v1" "prometheus" {
  metadata {
    name      = "prometheus-config"
    namespace = "monitoring"
  }

  data = {
    "prometheus.yml" = <<EOF
global:
  scrape_interval: 15s
rule_files:
  - "/etc/prometheus/alerts.yml"
scrape_configs:
  - job_name: app
    kubernetes_sd_configs:
      - role: pod
    relabel_configs:
      - source_labels: [__meta_kubernetes_pod_label_app]
        regex: app
        action: keep
      - source_labels: [__address__]
        regex: (.+):\d+
        replacement: $1:8080
        target_label: __address__

  - job_name: node-exporter
    kubernetes_sd_configs:
      - role: pod
    relabel_configs:
      - source_labels: [__meta_kubernetes_pod_label_app]
        regex: node-exporter
        action: keep

  - job_name: kube-state-metrics
    kubernetes_sd_configs:
      - role: pod
    relabel_configs:
      - source_labels: [__meta_kubernetes_pod_label_app]
        regex: kube-state-metrics-server
        action: keep
EOF
    "alerts.yml"     = <<EOF
groups:
  - name: app
    rules:
      - alert: AppDown
        expr: up{app="app"} == 0
        for: 1m
        labels:
          severity: critical
        annotations:
          summary: "App is down"
      - alert: HighCPU
        expr: rate(process_cpu_seconds_total{app="app"}[5m]) > 0.8
        for: 2m
        labels:
          severity: warning
EOF
  }
}

resource "kubernetes_deployment_v1" "prometheus" {
  metadata {
    name      = "prometheus-deployment"
    namespace = "monitoring"
    labels = {
      app = "prometheus-server"
    }
  }
  spec {
    replicas = 1
    
    selector {
      match_labels = {
        app = "prometheus-server"
      }
    }

    template {
      metadata {
        labels = {
          app = "prometheus-server"
        }
      }
      spec {
        container {
          name  = "prometheus"
          image = "prom/prometheus"
          args = ["--config.file=/etc/prometheus/prometheus.yml",
            "--storage.tsdb.retention.time=7d",
          "--storage.tsdb.path=/prometheus"]
          port {
            container_port = 9090

          }
          volume_mount {
            name       = "config"
            mount_path = "/etc/prometheus/"
          }
          volume_mount {
            name       = "data"
            mount_path = "/prometheus"
          }
          resources {
            limits = {
              memory = "150Mi"
              cpu    = "100m"
            }
          }
        }
        enable_service_links            = false

        volume {
          name = "config"
          config_map {
            name = "prometheus-config"
          }
        }
        volume {
          name = "data"
          persistent_volume_claim {
            claim_name = "prometheus-data"
          }
        }
      }
    }
  }
}