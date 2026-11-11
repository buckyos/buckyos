import json, sys, time
import dv_gate as d
E = sys.argv[1]
req = {
    "key": f"dv-broken-{int(time.time())}",
    "title": "DV: broken check",
    "objective": f"Whenever a new file appears in {d.INBOX}, append its name and size as one line to {d.AGENT_ROOT}/dv_seen.txt. Keep your check script working: when it fails, fix it.",
    "lifecycle": "maintain",
    "healthy_state": "every file in dv_inbox is listed in dv_seen.txt",
    "distance_evaluation": "files in dv_inbox not yet in dv_seen.txt",
    "schedule": {"every_secs": 60, "min_interval_secs": 60, "guarantee_secs": 600},
    "budget": {"activation": {"max_total_tokens": 200000, "max_runs": 6}, "period": {"max_total_tokens": 400000, "max_works": 0}},
    "check_script": d.BROKEN,
}
g = d.call("goals.create", {"request": req})
gid = g["goal_id"]
open(f"{E}/broken_goal_id.txt", "w").write(gid)
out = open(f"{E}/05-broken-check.jsonl", "a")
deadline = time.time() + 1500
archived = False
while time.time() < deadline:
    s = d.call("goals.schedule", {"goal_id": gid}) or {}
    acts = d.call("goals.activations", {"goal_id": gid, "query": {"limit": 0}}) or []
    g = d.call("goals.get", {"goal_id": gid})
    rec = {"t": time.strftime("%H:%M:%S"), "check": g["schedule"].get("check"), "last_check": s.get("last_check"),
           "check_errors": s.get("check_errors"), "last_check_error": (s.get("last_check_error") or "")[:200],
           "stats": s.get("stats"), "activations": [{"seq": a["seq"], "reasons": [(r["kind"], (r.get("note") or "")[:200]) for r in a["reasons"]],
           "decision": a["decision"], "outcome": a["outcome"], "tokens": a["cost"]["total_tokens"], "summary": (a.get("summary") or "")[:400]} for a in acts]}
    out.write(json.dumps(rec, ensure_ascii=False) + "\n"); out.flush()
    if acts and not archived:
        # The first (guarantee) Activation is over: watch two more checks, then stop it.
        time.sleep(130)
        s = d.call("goals.schedule", {"goal_id": gid}) or {}
        g = d.call("goals.get", {"goal_id": gid})
        out.write(json.dumps({"t": time.strftime("%H:%M:%S"), "label": "after fix", "check": g["schedule"].get("check"), "last_check": s.get("last_check"), "check_errors": s.get("check_errors"), "stats": s.get("stats")}, ensure_ascii=False) + "\n")
        d.call("goals.set_status", {"goal_id": gid, "request": {"status": "archived", "reason": "DV done"}})
        out.write(json.dumps({"t": time.strftime("%H:%M:%S"), "label": "archived"}) + "\n"); out.flush()
        archived = True
        break
    time.sleep(30)
if not archived:
    d.call("goals.set_status", {"goal_id": gid, "request": {"status": "archived", "reason": "DV timeout"}})
    out.write(json.dumps({"t": time.strftime("%H:%M:%S"), "label": "archived on timeout"}) + "\n")
print("done", gid)
