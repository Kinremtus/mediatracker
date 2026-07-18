resource "kubernetes_cluster_role_v1" "monitoring-rbac" {
  metadata {
    name = "monitoring"
  }
  rule {
    api_groups = [""]
    resources = ["pods", "services", "nodes"]
    verbs = ["get", "list", "watch"]
  }
  rule {
    api_groups = ["apps"]
    resources = ["deployments", "daemonsets", "statefulsets"]
    verbs = ["list", "watch"]
  }
}

resource "kubernetes_cluster_role_binding_v1" "monitoring-rbac" {
  metadata {
    name = "monitoring"
  }
  role_ref {
    api_group = "rbac.authorization.k8s.io"
    kind = "ClusterRole"
    name = "monitoring"
  }
  subject {
    kind = "ServiceAccount"
    name = "default"
    namespace = "monitoring"
  }
}
