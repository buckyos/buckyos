# OpenDAN Tools Reference

TODO： bash命令需要配合一个runtimne filesystem的解释说明，鼓励agent仅仅使用元工具就能完成各种工作
- memory的文件系统结构
- message_record的文件系统结构
- worklog的文件系统结构
- workspace summary的文件系统结构
- session summary的文件系统结构

================ TOOL PROMPTS ================
[List Mode] name + summary

- create_sub_agent : Create a sub-agent execution session.
- edit : Edit file by anchor.
- exec : Run shell command.
- read : Read file.
- write : Write file.

- get_session : Get current session detail. (可以去掉)
- bind_external_workspace : Bind an external workspace to current session.
- list_external_workspaces : List bindable external workspaces.
- list_session : List available sessions.
- load_memory : Load memory entries by query and scope.

- todo_manage : Manage workspace todos.
- worklog_manage : Write or query worklog records. （可以去掉)




[Detail Mode] one tool spec per block

### TOOL bind_external_workspace
{
  "name": "bind_external_workspace",
  "description": "Bind an external workspace to current session.",
  "args_schema": {
    "properties": {
      "workspace_id": {
        "type": "string"
      }
    },
    "required": [
      "workspace_id"
    ],
    "type": "object"
  },
  "output_schema": {
    "type": "object"
  }
}

### TOOL create_sub_agent
{
  "name": "create_sub_agent",
  "description": "Create a sub-agent execution session.",
  "args_schema": {
    "properties": {
      "goal": {
        "type": "string"
      },
      "role": {
        "type": "string"
      }
    },
    "type": "object"
  },
  "output_schema": {
    "type": "object"
  }
}

### TOOL edit
{
  "name": "edit",
  "description": "Edit file by anchor.",
  "args_schema": {
    "properties": {
      "mode": {
        "enum": [
          "replace",
          "after",
          "before"
        ],
        "type": "string"
      },
      "new_content": {
        "type": "string"
      },
      "path": {
        "type": "string"
      },
      "pos_chunk": {
        "type": "string"
      }
    },
    "required": [
      "path",
      "pos_chunk",
      "new_content",
      "mode"
    ],
    "type": "object"
  },
  "output_schema": {
    "type": "object"
  }
}

### TOOL exec
{
  "name": "exec",
  "description": "Run shell command.",
  "args_schema": {
    "properties": {
      "command": {
        "type": "string"
      },
      "env": {
        "additionalProperties": {
          "type": "string"
        },
        "type": "object"
      },
      "timeout_ms": {
        "minimum": 1,
        "type": "integer"
      }
    },
    "required": [
      "command"
    ],
    "type": "object"
  },
  "output_schema": {
    "type": "object"
  }
}

### TOOL get_session
{
  "name": "get_session",
  "description": "Get current session detail.",
  "args_schema": {
    "properties": {
      "session_id": {
        "type": "string"
      }
    },
    "type": "object"
  },
  "output_schema": {
    "type": "object"
  }
}

### TOOL list_external_workspaces
{
  "name": "list_external_workspaces",
  "description": "List bindable external workspaces.",
  "args_schema": {
    "properties": {
      "provider": {
        "type": "string"
      }
    },
    "type": "object"
  },
  "output_schema": {
    "type": "object"
  }
}

### TOOL list_session
{
  "name": "list_session",
  "description": "List available sessions.",
  "args_schema": {
    "properties": {
      "limit": {
        "minimum": 1,
        "type": "integer"
      }
    },
    "type": "object"
  },
  "output_schema": {
    "type": "object"
  }
}

### TOOL load_memory
{
  "name": "load_memory",
  "description": "Load memory entries by query and scope.",
  "args_schema": {
    "properties": {
      "limit": {
        "minimum": 1,
        "type": "integer"
      },
      "query": {
        "type": "string"
      },
      "scope": {
        "type": "string"
      }
    },
    "type": "object"
  },
  "output_schema": {
    "type": "object"
  }
}

### TOOL read
{
  "name": "read",
  "description": "Read file.",
  "args_schema": {
    "properties": {
      "first_chunk": {
        "type": "string"
      },
      "path": {
        "type": "string"
      },
      "range": {}
    },
    "required": [
      "path"
    ],
    "type": "object"
  },
  "output_schema": {
    "type": "object"
  }
}

### TOOL todo_manage
{
  "name": "todo_manage",
  "description": "Manage workspace todos.",
  "args_schema": {
    "properties": {
      "ops": {
        "type": "array"
      },
      "workspace_id": {
        "type": "string"
      }
    },
    "required": [
      "ops"
    ],
    "type": "object"
  },
  "output_schema": {
    "type": "object"
  }
}

### TOOL worklog_manage
{
  "name": "worklog_manage",
  "description": "Write or query worklog records.",
  "args_schema": {
    "properties": {
      "op": {
        "type": "string"
      },
      "workspace_id": {
        "type": "string"
      }
    },
    "required": [
      "op"
    ],
    "type": "object"
  },
  "output_schema": {
    "type": "object"
  }
}

### TOOL write
{
  "name": "write",
  "description": "Write file.",
  "args_schema": {
    "properties": {
      "content": {
        "type": "string"
      },
      "mode": {
        "enum": [
          "new",
          "append",
          "write"
        ],
        "type": "string"
      },
      "path": {
        "type": "string"
      }
    },
    "required": [
      "path",
      "content",
      "mode"
    ],
    "type": "object"
  },
  "output_schema": {
    "type": "object"
  }
}

[Prompt Payload] ToolSpec::render_for_prompt output
[{"name":"bind_external_workspace","description":"Bind an external workspace to current session.","args_schema":{"properties":{"workspace_id":{"type":"string"}},"required":["workspace_id"],"type":"object"},"output_schema":{"type":"object"}},{"name":"create_sub_agent","description":"Create a sub-agent execution session.","args_schema":{"properties":{"goal":{"type":"string"},"role":{"type":"string"}},"type":"object"},"output_schema":{"type":"object"}},{"name":"edit","description":"Edit file by anchor.","args_schema":{"properties":{"mode":{"enum":["replace","after","before"],"type":"string"},"new_content":{"type":"string"},"path":{"type":"string"},"pos_chunk":{"type":"string"}},"required":["path","pos_chunk","new_content","mode"],"type":"object"},"output_schema":{"type":"object"}},{"name":"exec","description":"Run shell command.","args_schema":{"properties":{"command":{"type":"string"},"env":{"additionalProperties":{"type":"string"},"type":"object"},"timeout_ms":{"minimum":1,"type":"integer"}},"required":["command"],"type":"object"},"output_schema":{"type":"object"}},{"name":"get_session","description":"Get current session detail.","args_schema":{"properties":{"session_id":{"type":"string"}},"type":"object"},"output_schema":{"type":"object"}},{"name":"list_external_workspaces","description":"List bindable external workspaces.","args_schema":{"properties":{"provider":{"type":"string"}},"type":"object"},"output_schema":{"type":"object"}},{"name":"list_session","description":"List available sessions.","args_schema":{"properties":{"limit":{"minimum":1,"type":"integer"}},"type":"object"},"output_schema":{"type":"object"}},{"name":"load_memory","description":"Load memory entries by query and scope.","args_schema":{"properties":{"limit":{"minimum":1,"type":"integer"},"query":{"type":"string"},"scope":{"type":"string"}},"type":"object"},"output_schema":{"type":"object"}},{"name":"read","description":"Read file.","args_schema":{"properties":{"first_chunk":{"type":"string"},"path":{"type":"string"},"range":{}},"required":["path"],"type":"object"},"output_schema":{"type":"object"}},{"name":"todo_manage","description":"Manage workspace todos.","args_schema":{"properties":{"ops":{"type":"array"},"workspace_id":{"type":"string"}},"required":["ops"],"type":"object"},"output_schema":{"type":"object"}},{"name":"worklog_manage","description":"Write or query worklog records.","args_schema":{"properties":{"op":{"type":"string"},"workspace_id":{"type":"string"}},"required":["op"],"type":"object"},"output_schema":{"type":"object"}},{"name":"write","description":"Write file.","args_schema":{"properties":{"content":{"type":"string"},"mode":{"enum":["new","append","write"],"type":"string"},"path":{"type":"string"}},"required":["path","content","mode"],"type":"object"},"output_schema":{"type":"object"}}]

================ ACTION PROMPTS ================
[List Mode] name + introduce
- bind_external_workspace : Bind an external workspace to current session.
- create_sub_agent : Create a sub-agent execution session.
- edit : Edit file by anchor.
- exec : Run shell command.
- get_session : Get current session detail.
- list_external_workspaces : List bindable external workspaces.
- list_session : List available sessions.
- load_memory : Load memory entries by query and scope.
- read : Read file.
- todo_manage : Manage workspace todos.
- worklog_manage : Write or query worklog records.
- write : Write file.

[Detail Mode] one action prompt per block

### ACTION bind_external_workspace
**bind_external_workspace**
 - Action Name: bind_external_workspace
 - Kind: call_tool
 - Usage: ["bind_external_workspace", {"properties":{"workspace_id":{"type":"string"}},"required":["workspace_id"],"type":"object"}]
 - Description: Bind an external workspace to current session. Args schema: {"properties":{"workspace_id":{"type":"string"}},"required":["workspace_id"],"type":"object"}

### ACTION create_sub_agent
**create_sub_agent**
 - Action Name: create_sub_agent
 - Kind: call_tool
 - Usage: ["create_sub_agent", {"properties":{"goal":{"type":"string"},"role":{"type":"string"}},"type":"object"}]
 - Description: Create a sub-agent execution session. Args schema: {"properties":{"goal":{"type":"string"},"role":{"type":"string"}},"type":"object"}

### ACTION edit
**edit**
 - Action Name: edit
 - Kind: call_tool
 - Usage: ["edit", {"properties":{"mode":{"enum":["replace","after","before"],"type":"string"},"new_content":{"type":"string"},"path":{"type":"string"},"pos_chunk":{"type":"string"}},"required":["path","pos_chunk","new_content","mode"],"type":"object"}]
 - Description: Edit file by anchor. Args schema: {"properties":{"mode":{"enum":["replace","after","before"],"type":"string"},"new_content":{"type":"string"},"path":{"type":"string"},"pos_chunk":{"type":"string"}},"required":["path","pos_chunk","new_content","mode"],"type":"object"}

### ACTION exec
**exec**
 - Action Name: exec
 - Kind: call_tool
 - Usage: ["exec", {"properties":{"command":{"type":"string"},"env":{"additionalProperties":{"type":"string"},"type":"object"},"timeout_ms":{"minimum":1,"type":"integer"}},"required":["command"],"type":"object"}]
 - Description: Run shell command. Args schema: {"properties":{"command":{"type":"string"},"env":{"additionalProperties":{"type":"string"},"type":"object"},"timeout_ms":{"minimum":1,"type":"integer"}},"required":["command"],"type":"object"}

### ACTION get_session
**get_session**
 - Action Name: get_session
 - Kind: call_tool
 - Usage: ["get_session", {"properties":{"session_id":{"type":"string"}},"type":"object"}]
 - Description: Get current session detail. Args schema: {"properties":{"session_id":{"type":"string"}},"type":"object"}

### ACTION list_external_workspaces
**list_external_workspaces**
 - Action Name: list_external_workspaces
 - Kind: call_tool
 - Usage: ["list_external_workspaces", {"properties":{"provider":{"type":"string"}},"type":"object"}]
 - Description: List bindable external workspaces. Args schema: {"properties":{"provider":{"type":"string"}},"type":"object"}

### ACTION list_session
**list_session**
 - Action Name: list_session
 - Kind: call_tool
 - Usage: ["list_session", {"properties":{"limit":{"minimum":1,"type":"integer"}},"type":"object"}]
 - Description: List available sessions. Args schema: {"properties":{"limit":{"minimum":1,"type":"integer"}},"type":"object"}

### ACTION load_memory
**load_memory**
 - Action Name: load_memory
 - Kind: call_tool
 - Usage: ["load_memory", {"properties":{"limit":{"minimum":1,"type":"integer"},"query":{"type":"string"},"scope":{"type":"string"}},"type":"object"}]
 - Description: Load memory entries by query and scope. Args schema: {"properties":{"limit":{"minimum":1,"type":"integer"},"query":{"type":"string"},"scope":{"type":"string"}},"type":"object"}

### ACTION read
**read**
 - Action Name: read
 - Kind: call_tool
 - Usage: ["read", {"properties":{"first_chunk":{"type":"string"},"path":{"type":"string"},"range":{}},"required":["path"],"type":"object"}]
 - Description: Read file. Args schema: {"properties":{"first_chunk":{"type":"string"},"path":{"type":"string"},"range":{}},"required":["path"],"type":"object"}

### ACTION todo_manage
**todo_manage**
 - Action Name: todo_manage
 - Kind: call_tool
 - Usage: ["todo_manage", {"properties":{"ops":{"type":"array"},"workspace_id":{"type":"string"}},"required":["ops"],"type":"object"}]
 - Description: Manage workspace todos. Args schema: {"properties":{"ops":{"type":"array"},"workspace_id":{"type":"string"}},"required":["ops"],"type":"object"}

### ACTION worklog_manage
**worklog_manage**
 - Action Name: worklog_manage
 - Kind: call_tool
 - Usage: ["worklog_manage", {"properties":{"op":{"type":"string"},"workspace_id":{"type":"string"}},"required":["op"],"type":"object"}]
 - Description: Write or query worklog records. Args schema: {"properties":{"op":{"type":"string"},"workspace_id":{"type":"string"}},"required":["op"],"type":"object"}

### ACTION write
**write**
 - Action Name: write
 - Kind: call_tool
 - Usage: ["write", {"properties":{"content":{"type":"string"},"mode":{"enum":["new","append","write"],"type":"string"},"path":{"type":"string"}},"required":["path","content","mode"],"type":"object"}]
 - Description: Write file. Args schema: {"properties":{"content":{"type":"string"},"mode":{"enum":["new","append","write"],"type":"string"},"path":{"type":"string"}},"required":["path","content","mode"],"type":"object"}
test agent_tool::tests::print_tool_and_action_prompt_catalog_for_review ... ok