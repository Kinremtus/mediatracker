resource "kubernetes_service_v1" "kube-state-metrics" {
    metadata {
      name      = "kube-state-metrics"
      namespace = "monitoring"
    }
    spec {
      selector = {
        app = "kube-state-metrics-server"
      }
      port {
        port = 8080
        target_port = 8080
      }
    } 
}

resource "kubernetes_service_account_v1" "kube-state-metrics" {
  metadata {
    name = "kube-state-metrics"
    namespace = "monitoring"
  }
}

resource "kubernetes_cluster_role_v1" "kube-state-metrics" {
  metadata {
    name = "kube-state-metrics"
  }
  rule {
    api_groups = [""]
    resources = ["nodes", "pods", "services", "persistentvolumeclaims"]
    verbs = ["list", "watch"]
  }
  rule {
    api_groups = ["apps"]
    resources = ["deployments", "daemonsets", "statefulsets"]
    verbs = ["list", "watch"]
  }
}

resource "kubernetes_cluster_role_binding_v1" "kube-state-metrics" {
  metadata {
    name = "kube-state-metrics"
  }
  role_ref {
    api_group = "rbac.authorization.k8s.io"
    kind = "ClusterRole"
    name = "kube-state-metrics"
  }
  subject {
    kind = "ServiceAccount"
    name = "kube-state-metrics"
    namespace = "monitoring"
  }
}

resource "kubernetes_deployment_v1" "kube-state-metrics" {
  metadata {
    name = "kube-state-metrics-deployment"
    namespace = "monitoring"
    labels = {
      app = "kube-state-metrics-server"
    }
  }
  spec {
    replicas = 1
    selector {
      match_labels = {
        app = "kube-state-metrics-server"
      }
    }
    template {
      metadata {
        labels = {
          app = "kube-state-metrics-server"
        }
      }
      spec {
        service_account_name = "kube-state-metrics"
        container {
          name = "kube-state-metrics"
          image = "registry.k8s.io/kube-state-metrics/kube-state-metrics:v2.19.1"
          port {
            container_port = 8080
          }
          resources {
            limits = {
              memory = "40Mi"
              cpu = "30m"
            }
          }
        }
      }
    }
  }
}