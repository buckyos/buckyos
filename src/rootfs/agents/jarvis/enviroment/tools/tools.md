# AgentWorkshop Tools

OpenDAN runtime may provide many tools.
This workshop only enables a subset and defines per-tool boundaries.

```json
{
  "enabled_tools": [
    {
      "name": "exec_bash",
      "kind": "builtin",
      "enabled": true,
      "params": {
        "default_timeout_ms": 15000,
        "max_timeout_ms": 120000,
        "allow_env": false,
        "allowed_cwd_roots": [".", "todo", "artifacts", "tools"]
      }
    },
    {
      "name": "edit_file",
      "kind": "builtin",
      "enabled": true,
      "params": {
        "allow_create": true,
        "allow_replace": true,
        "max_write_bytes": 262144,
        "max_diff_lines": 200,
        "allowed_write_roots": ["todo", "artifacts", "tools", "worklog"]
      }
    }
  ]
}
```
