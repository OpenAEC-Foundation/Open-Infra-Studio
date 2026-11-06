# Known bugs

Follow-up items from reviews that do not block a merge. Each item names the PR it came from.

1. **MCP over HTTP: HEAD, TRACE, CONNECT and unknown methods skip the Host and Origin check** (#101). `BaseHTTPRequestHandler` answers them with 501 before `HttpHandler._refusal()` runs (`mcp/open_infra_mcp.py`). No body is read and no tool runs, so nothing can be reached this way, but the check does not run on every request. Fix: run the check before the method dispatch, and test HEAD and an unknown method.
