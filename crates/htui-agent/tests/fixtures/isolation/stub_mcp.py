"""A stub of htui's MCP server for tests/isolation_live.rs (MOD-91 T6, blueprint B8).

Serves one tool, `permission_prompt`, over newline-delimited JSON-RPC on stdio, and allows every
call it is asked. Appends each method it receives, each call's arguments and the `HTUI_SESSION`
value in its environment to the log file named by argv[1].
"""
import json
import os
import sys

log = open(sys.argv[1], "a")
log.write("env HTUI_SESSION=" + os.environ.get("HTUI_SESSION", "unset") + "\n")
log.flush()


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


PROMPT_TOOL = {
    "name": "permission_prompt",
    "description": "permission prompt",
    "inputSchema": {
        "type": "object",
        "properties": {
            "tool_name": {"type": "string"},
            "input": {"type": "object"},
            "tool_use_id": {"type": "string"},
        },
        "required": ["tool_name", "input"],
    },
}

for line in sys.stdin:
    try:
        message = json.loads(line)
    except ValueError:
        continue
    mid = message.get("id")
    method = message.get("method")
    log.write(str(method) + "\n")
    log.flush()
    if method == "initialize":
        version = message["params"].get("protocolVersion", "2025-06-18")
        send({"jsonrpc": "2.0", "id": mid, "result": {
            "protocolVersion": version,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "htui", "version": "0"},
        }})
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": mid, "result": {"tools": [PROMPT_TOOL]}})
    elif method == "tools/call":
        args = message["params"].get("arguments", {})
        log.write("CALL " + json.dumps(args) + "\n")
        log.flush()
        verdict = {"behavior": "allow", "updatedInput": args.get("input", {})}
        send({"jsonrpc": "2.0", "id": mid, "result": {
            "content": [{"type": "text", "text": json.dumps(verdict)}],
        }})
    elif mid is not None:
        send({"jsonrpc": "2.0", "id": mid, "result": {}})
