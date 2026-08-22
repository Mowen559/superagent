import sys
import json
import time
import urllib.request
import urllib.parse

def call_agent(prompt, role):
    # In a real dynamic tool, we would call the local hub-rs API or use the subagent system.
    # For now, we simulate the subagent execution or print the orchestration steps.
    print(f"[Workflow Engine] Spawning Subagent - Role: {role}")
    print(f"[Workflow Engine] Prompt: {prompt}")
    # Simulate work
    time.sleep(2)
    print(f"[Workflow Engine] Subagent {role} completed.")
    return f"Result of {role}"

def run_workflow(payload_str):
    try:
        workflow = json.loads(payload_str)
    except Exception as e:
        return {"error": f"Invalid JSON payload: {e}"}
    
    tasks = workflow.get("tasks", [])
    name = workflow.get("name", "Unnamed Workflow")
    
    print(f"=== Starting Workflow: {name} ===")
    
    # Build dependency graph
    task_map = {t["id"]: t for t in tasks}
    in_degree = {t["id"]: len(t.get("depends_on", [])) for t in tasks}
    adj = {t["id"]: [] for t in tasks}
    
    for t in tasks:
        for dep in t.get("depends_on", []):
            if dep in adj:
                adj[dep].append(t["id"])
                
    # Topological sort / execution queue
    queue = [tid for tid, deg in in_degree.items() if deg == 0]
    completed = {}
    
    while queue:
        current_id = queue.pop(0)
        task = task_map[current_id]
        
        # Execute task
        result = call_agent(task["prompt"], task["role"])
        completed[current_id] = result
        
        # Resolve dependencies
        for neighbor in adj[current_id]:
            in_degree[neighbor] -= 1
            if in_degree[neighbor] == 0:
                queue.append(neighbor)
                
    if len(completed) < len(tasks):
        return {"status": "error", "message": "Cycle detected or missing dependencies in workflow graph."}
        
    print(f"=== Workflow {name} Completed ===")
    return {"status": "success", "completed_tasks": len(completed), "results": completed}

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(json.dumps({"error": "Missing payload"}))
        sys.exit(1)
    
    payload = sys.argv[1]
    result = run_workflow(payload)
    print(json.dumps(result))
