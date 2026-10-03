use serde_json::Value;
use std::path::Path;

pub fn rewrite(root: &Path, replacements: &[(&str, &str)]) {
    let mut stack = vec![root.to_path_buf()];
    let mut states = Vec::new();
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if let Ok(mut s) = std::fs::read_to_string(&p) {
                for (from, to) in replacements {
                    if p.extension().is_some_and(|ext| ext == "json" || ext == "jsonl") {
                        let from = serde_json::to_string(from).unwrap();
                        let to = serde_json::to_string(to).unwrap();
                        s = s.replace(&from[1..from.len() - 1], &to[1..to.len() - 1]);
                    } else {
                        s = s.replace(from, to);
                    }
                }
                std::fs::write(&p, s).unwrap();
            }
            if p.file_name().is_some_and(|n| n == "state.json")
                && p.parent()
                    .and_then(Path::file_name)
                    .is_some_and(|n| n == ".opendan_agent_session")
            {
                states.push(p);
            }
        }
    }
    let expected_path = root.join("expected.json");
    let mut expected: Value = read(&expected_path);
    for p in states {
        let dir = p.parent().unwrap();
        let log = dir.join("worklog.jsonl");
        let bytes = std::fs::read(&log).unwrap();
        let mut offset = 0u64;
        let mut boundaries = Vec::new();
        for line in bytes.split_inclusive(|b| *b == b'\n') {
            if let Ok(v) = serde_json::from_slice::<Value>(line) {
                if let Some(seq) = v["seq"].as_u64() {
                    boundaries.push((seq, offset, offset + line.len() as u64));
                }
            }
            offset += line.len() as u64;
        }
        let mut state = read(&p);
        let seq = state["worklog"]["committed_seq"].as_u64().unwrap();
        let committed = boundaries
            .iter()
            .find(|(s, _, _)| *s == seq)
            .map(|(_, _, end)| *end)
            .unwrap_or(0);
        state["worklog"]["committed_bytes"] = committed.into();
        write(&p, &state);
        let summary = dir.join("summary.json");
        if summary.exists() {
            let mut v = read(&summary);
            let start = v["start_seq"].as_u64().unwrap_or(0);
            let start_offset = boundaries
                .iter()
                .find(|(s, _, _)| *s == start)
                .map(|(_, at, _)| *at)
                .unwrap_or(0);
            v["start_offset"] = start_offset.into();
            write(&summary, &v);
            if !expected["next"]["stop_at_offset"].is_null() {
                expected["next"]["stop_at_offset"] = start_offset.into();
            }
        }
        let sid = dir.parent().unwrap().file_name().unwrap().to_string_lossy();
        if let Some(sessions) = expected["sessions"].as_array_mut() {
            for entry in sessions
                .iter_mut()
                .filter(|e| e["session_id"].as_str() == Some(&sid))
            {
                entry["worklog"]["committed_bytes"] = committed.into();
                entry["worklog"]["file_bytes"] = (bytes.len() as u64).into();
                entry["worklog"]["uncommitted_tail"] = (bytes.len() as u64 > committed).into();
            }
        }
    }
    if expected_path.exists() {
        write(&expected_path, &expected);
    }
}
fn read(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}
fn write(path: &Path, v: &Value) {
    std::fs::write(path, serde_json::to_vec_pretty(v).unwrap()).unwrap();
}
pub fn uid() -> String {
    String::from_utf8_lossy(
        &std::process::Command::new("id")
            .arg("-u")
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .to_string()
}
pub fn hostname() -> String {
    String::from_utf8_lossy(
        &std::process::Command::new("hostname")
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .to_string()
}
