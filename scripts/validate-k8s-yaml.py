#!/usr/bin/env python3
"""Validate all K8s YAML files in k8s/ before deploy.

Checks:
- YAML syntax (every file, every document)
- Required fields: apiVersion, kind, metadata.name, metadata.namespace
- Service names referenced by IngressRoute exist
- Port mismatches between IngressRoute and target Service

Usage: python3 scripts/validate-k8s-yaml.py
Exit 0 = OK, 1 = errors found.
"""

import os
import sys
import yaml
import glob


K8S_DIR = "k8s"
NAMESPACE = "mediatracker"

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
    """Parse all YAML files, return list of (file, docs)."""
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
    """Build {name: {namespace, port}} from Service resources."""
    services = {}
    for fpath, docs in parsed:
        for doc in docs:
            if doc is None:
                continue
            if doc.get("kind") == "Service":
                meta = doc.get("metadata", {})
                name = meta.get("name")
                ns = meta.get("namespace", NAMESPACE)
                ports = doc.get("spec", {}).get("ports", [])
                if name:
                    services[name] = {
                        "namespace": ns,
                        "port": ports[0]["port"] if ports else None,
                        "file": fpath,
                    }
    return services


def validate_syntax(parsed):
    """All files should parse as valid YAML (handled in parse_yaml_files)."""
    pass


def validate_required_fields(parsed):
    for fpath, docs in parsed:
        for i, doc in enumerate(docs):
            if doc is None:
                continue
            tag = f"{fpath}:doc{i}"
            required_fields(doc, tag, ["apiVersion", "kind", "metadata.name"])


SYSTEM_NAMESPACES = {"kube-system", "kube-public", "kube-node-lease"}
CLUSTER_SCOPED = {
    "Namespace", "ClusterRole", "ClusterRoleBinding",
    "StorageClass", "VolumeSnapshotClass",
}


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
            if ns in SYSTEM_NAMESPACES:
                continue
            if ns != NAMESPACE:
                if ns is None:
                    err(f"Missing namespace (expected '{NAMESPACE}')", f"{fpath}:doc{i}")
                elif ns:
                    err(f"Wrong namespace '{ns}' (expected '{NAMESPACE}')", f"{fpath}:doc{i}")


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
                        if svc_name:
                            if svc_name not in services:
                                err(
                                    f"IngressRoute references service '{svc_name}' "
                                    f"but no Service with that name exists",
                                    f"{fpath}:doc{i}",
                                )
                            else:
                                actual = services[svc_name]
                                actual_ns = actual["namespace"]
                                if actual_ns != doc.get("metadata", {}).get("namespace"):
                                    warn(
                                        f"Service '{svc_name}' is in namespace "
                                        f"'{actual_ns}' but IngressRoute is in "
                                        f"'{doc.get('metadata', {}).get('namespace')}'",
                                        f"{fpath}:doc{i}",
                                    )
                                if svc_port and actual["port"] and svc_port != actual["port"]:
                                    err(
                                        f"IngressRoute port {svc_port} but "
                                        f"Service '{svc_name}' exposes port {actual['port']}",
                                        f"{fpath}:doc{i}",
                                    )


def main():
    parsed = parse_yaml_files()

    if errors:
        print("❌ YAML PARSE ERRORS — cannot continue")
        for e in errors:
            print(f"   {e}")
        sys.exit(1)

    services = build_service_map(parsed)

    validate_required_fields(parsed)
    validate_namespace(parsed)
    validate_ingress_routes(parsed, services)

    # Report
    yaml_count = sum(len(docs) for _, docs in parsed)
    file_count = len(parsed)
    print(f"Checked {file_count} files ({yaml_count} YAML documents)")

    if errors:
        print(f"\n❌ {len(errors)} error(s):")
        for e in errors:
            print(f"   {e}")
        sys.exit(1)

    if warnings:
        print(f"\n⚠️  {len(warnings)} warning(s):")
        for w in warnings:
            print(f"   {w}")

    print("\n✅ All K8s YAML files valid")
    sys.exit(0)


if __name__ == "__main__":
    main()
