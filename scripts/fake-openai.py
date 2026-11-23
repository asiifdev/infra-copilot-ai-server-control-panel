#!/usr/bin/env python3
"""A tiny OpenAI-compatible server for local UI testing. Loopback only, streams SSE.

Usage: fake-openai.py PORT KEY_FILE   (the Bearer key it accepts is the content of KEY_FILE)
Used by scripts/demo-serve.sh --fake-ai. It is a test double, not part of the product.

Round 1 (a request with tools and no tool message yet): calls the tool `list_nodes`.
Round 2 (a tool message is present): answers in text that mentions what the tool returned.
A request without tools: answers `pong`. A wrong key gets 401.
"""
import json
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

KEY_FILE = sys.argv[2]
EXPECTED = open(KEY_FILE).read().strip()


def sse(handler, chunks):
    handler.send_response(200)
    handler.send_header("Content-Type", "text/event-stream")
    handler.send_header("Cache-Control", "no-cache")
    handler.end_headers()
    for chunk in chunks:
        handler.wfile.write(("data: " + chunk + "\n\n").encode())
        handler.wfile.flush()
        time.sleep(0.05)


def delta(text, finish=None):
    return json.dumps({"model": "fake-1", "choices": [{"delta": {"content": text}, "finish_reason": finish}]})


class Handler(BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        pass

    def do_POST(self):
        if self.headers.get("Authorization", "") != "Bearer " + EXPECTED:
            self.send_response(401)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"error":{"message":"bad key"}}')
            return
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))))
        messages = body.get("messages", [])
        tools = body.get("tools", [])
        tool_msgs = [m for m in messages if m.get("role") == "tool"]
        if not tools:
            sse(self, [delta("pong", "stop"), "[DONE]"])
        elif not tool_msgs:
            call = {
                "model": "fake-1",
                "choices": [{
                    "delta": {"tool_calls": [{"index": 0, "id": "call_1", "type": "function",
                                              "function": {"name": "list_nodes", "arguments": "{}"}}]},
                    "finish_reason": None}],
            }
            fin = {"choices": [{"delta": {}, "finish_reason": "tool_calls"}]}
            sse(self, [json.dumps(call), json.dumps(fin), "[DONE]"])
        else:
            seen = tool_msgs[-1].get("content", "")
            names = [n for n in ("old-server", "new-server") if n in seen]
            text = "You have " + str(len(names)) + " servers: " + ", ".join(names) + ".\n"
            text += "<b>This is not markup</b> <script>alert(1)</script>"
            sse(self, [delta(text[:20]), delta(text[20:], "stop"), "[DONE]"])


ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
