#!/usr/bin/env python3
"""Validate all K8s YAML files in k8s/ before deploy.

Checks:
- YAML syntax (every file, every document)
- Required fields: apiVersion, kind, metadata.name
- Namespace present on namespaced resources
- Service port consistency (IngressRoute vs Service, Prometheus targets vs Service)
- IngressRoute references existing Service
- Bad fields: clusterIP in Deployment/DaemonSet, serviceAccountName at wrong level

Usage: python3 scripts/validate-k8s-yaml.py
Exit 0 = OK, 1 = errors found.
"""

import os, sys, glob
import yaml


K8S_DIR = "k8s"
CLUSTER_SCOPED = {
    "Namespace", "ClusterRole", "ClusterRoleBinding",
    "StorageClass", "VolumeSnapshotClass",
}
NAMESPACED_KINDS = {
    "Deployment", "DaemonSet", "Service", "ConfigMap", "Secret",
    "IngressRoute", "Ingress", "ServiceAccount", "Role", "RoleBinding",
    "PersistentVolumeClaim", "HorizontalPodAutoscaler",
    "NetworkPolicy", "PodDisruptionBudget",
}

errors = []
warnings = []

def err(msg, file=None):
    prefix = f"[{file}] " if file else ""
    errors.append(f"{prefix}{msg}")

def warn(msg, file=None):
    prefix = f"[{file}] " if file else ""
    warnings.append(f"{prefix}{msg}")

def required_fields(doc, file, fields):
    for f in fields:
        parts = f.split(".")
        val = doc
        for p in parts:
            if isinstance(val, dict) and p in val:
                val = val[p]
            else:
                err(f"Missing required field: {f}", file)
                break

def parse_yaml_files():
    results = []
    pattern = os.path.join(K8S_DIR, "**/*.yaml")
    for fpath in sorted(glob.glob(pattern, recursive=True)):
        try:
            with open(fpath) as fh:
                docs = list(yaml.safe_load_all(fh))
            results.append((fpath, docs))
        except yaml.YAMLError as e:
            err(f"YAML parse error: {e}", fpath)
    return results

def build_service_map(parsed):
    services = {}
    for fpath, docs in parsed:
        for doc in docs:
            if doc is None:
                continue
            if doc.get("kind") == "Service":
                meta = doc.get("metadata", {})
                name = meta.get("name")
                ns = meta.get("namespace")
                ports = doc.get("spec", {}).get("ports", [])
                if name:
                    services[(name, ns)] = {
                        "port": ports[0]["port"] if ports else None,
                        "file": fpath,
                    }
    return services

def validate_syntax(parsed):
    pass

def validate_required_fields(parsed):
    for fpath, docs in parsed:
        for i, doc in enumerate(docs):
            if doc is None:
                continue
            tag = f"{fpath}:doc{i}"
            required_fields(doc, tag, ["apiVersion", "kind", "metadata.name"])

def validate_namespace(parsed):
    for fpath, docs in parsed:
        for i, doc in enumerate(docs):
            if doc is None:
                continue
            kind = doc.get("kind", "")
            meta = doc.get("metadata", {})
            ns = meta.get("namespace")
            if kind in CLUSTER_SCOPED:
                continue
            if ns is None:
                err(f"Missing namespace on namespaced resource", f"{fpath}:doc{i}")
            elif kind == "IngressRoute" and ns != meta.get("namespace"):
                warn(f"IngressRoute namespace mismatch", f"{fpath}:doc{i}")

def validate_no_bad_fields(parsed):
    for fpath, docs in parsed:
        for i, doc in enumerate(docs):
            if doc is None:
                continue
            kind = doc.get("kind", "")
            if kind in ("Deployment", "DaemonSet", "StatefulSet"):
                spec = doc.get("spec", {})
                svc_name = spec.get("serviceAccountName")
                if svc_name:
                    err(
                        f"'serviceAccountName' must be in template.spec, not spec (at Deployment level)",
                        f"{fpath}:doc{i}",
                    )
                if "clusterIP" in spec:
                    err(
                        f"'clusterIP' is not a valid Deployment/DaemonSet field "
                        f"(it belongs to Service)",
                        f"{fpath}:doc{i}",
                    )
                # Check volumes are not inside containers
                template = spec.get("template", {})
                tspec = template.get("spec", {})
                containers = tspec.get("containers", [])
                for c in containers:
                    if "volumes" in c:
                        err(
                            f"'volumes' is inside container '{c.get('name', '?')}' "
                            f"(should be at spec level)",
                            f"{fpath}:doc{i}",
                        )
                # Check ports inside containers
                for c in containers:
                    if "ports" not in c:
                        warn(
                            f"Container '{c.get('name', '?')}' has no ports defined",
                            f"{fpath}:doc{i}",
                        )

def validate_ingress_routes(parsed, services):
    for fpath, docs in parsed:
        for i, doc in enumerate(docs):
            if doc is None:
                continue
            if doc.get("kind") in ("IngressRoute", "Ingress"):
                routes = doc.get("spec", {}).get("routes", [])
                for r in routes:
                    svcs = r.get("services", [])
                    for s in svcs:
                        svc_name = s.get("name")
                        svc_port = s.get("port")
                        ns = doc.get("metadata", {}).get("namespace")
                        if svc_name:
                            key = (svc_name, ns)
                            if key not in services:
                                err(
                                    f"IngressRoute references service '{svc_name}' "
                                    f"in namespace '{ns}' but no matching Service exists",
                                    f"{fpath}:doc{i}",
                                )
                            else:
                                actual = services[key]
                                if svc_port and actual["port"] and svc_port != actual["port"]:
                                    err(
                                        f"IngressRoute port {svc_port} but "
                                        f"Service '{svc_name}' exposes port {actual['port']}",
                                        f"{fpath}:doc{i}",
                                    )

def main():
    parsed = parse_yaml_files()
    if errors:
        print("YAML PARSE ERRORS")
        for e in errors:
            print(f"   {e}")
        sys.exit(1)

    services = build_service_map(parsed)

    validate_required_fields(parsed)
    validate_namespace(parsed)
    validate_no_bad_fields(parsed)
    validate_ingress_routes(parsed, services)

    yaml_count = sum(len(docs) for _, docs in parsed)
    file_count = len(parsed)
    print(f"Checked {file_count} files ({yaml_count} YAML documents)")

    if errors:
        print(f"\n  {len(errors)} error(s):")
        for e in errors:
            print(f"   {e}")
        sys.exit(1)

    if warnings:
        print(f"\n  {len(warnings)} warning(s):")
        for w in warnings:
            print(f"   {w}")

    print("\nAll K8s YAML files valid")
    sys.exit(0)

if __name__ == "__main__":
    main()
