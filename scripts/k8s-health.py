import subprocess
import json
from datetime import datetime, timezone

time_now = datetime.now(timezone.utc)

json_string = subprocess.run(
    ["kubectl", "get", "pods", "-A", "-o", "json"],
    check=True,
    capture_output=True,
    text=True,
)

pods_data = json.loads(json_string.stdout)

rows = []
for pod in pods_data["items"]:
    name = pod["metadata"]["name"]
    ns = pod["metadata"]["namespace"]
    phase = pod["status"]["phase"]
    restarts = str(pod["status"]["containerStatuses"][0]["restartCount"])  # в строку
    ready = str(pod["status"]["containerStatuses"][0]["ready"])            # в строку
    error_reason = (
        pod["status"]
        .get("containerStatuses", [{}])[0]
        .get("state", {})
        .get("waiting", {})
        .get("reason", "")
    )    
    age = pod ["status"]["startTime"]
    time_pod = datetime.fromisoformat(age)
    time_delta = time_now - time_pod
    pod_days = time_delta.days
    pod_hours = time_delta.seconds // 3600
    pod_minutes = (time_delta.seconds % 3600) // 60
    pod_age = f"{pod_days}d {pod_hours}h {pod_minutes}m"
    
    rows.append({
        "ns": ns, "name": name, "phase": phase,
        "restarts": restarts, "ready": ready, "errors": error_reason, "age": pod_age
    })

column = {}
for col in rows[0].keys():
    max_data = max(len(str(r[col])) for r in rows)   
    max_col_name = len(col.upper())                   
    column[f"max_{col}"] = max(max_data, max_col_name) 

header_parts = []
for col in rows[0].keys():
    col_name = col.upper() 
    formatted_col = f"{col_name:^{column[f'max_{col}']}}"
    header_parts.append(formatted_col)
    
total_length = sum(column[f"max_{col}"] for col in rows[0].keys()) + (len(rows[0].keys()) - 1) * 3
print("+" + "-" * (total_length + 2) + "+")

print("| " + " | ".join(header_parts) + " |")

print("+" + "-" * (total_length + 2) + "+")

for row in rows:
    row_parts = []
    
    for col, value in row.items():
        formatted_value = f"{str(value):^{column[f'max_{col}']}}"
        row_parts.append(formatted_value)
        
    # Печатаем готовую строку таблицы
    print("| " + " | ".join(row_parts) + " |")

print("+" + "-" * (total_length + 2) + "+")

total = len(rows)
running = sum(1 for r in rows if r["phase"] == "Running")
not_ready = sum(1 for r in rows if r["phase"] != "Running" or r["ready"] == "False")
errors = sum(1 for r in rows if r["errors"])

print(f"\nSummary:")
print(f"  Total:   {total}")
print(f"  Running: {running}")
print(f"  Not Ready: {not_ready}")
print(f"  Errors:   {errors}")
