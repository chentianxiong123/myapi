#!/usr/bin/env python3
# mock 上游：假供应商/假模型/假流（实测用）
# 支持 /v1/chat/completions（OpenAI）、/v1/messages（Anthropic）、/v1/responses
from http.server import HTTPServer, BaseHTTPRequestHandler
import json, time, sys

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 19999

class H(BaseHTTPRequestHandler):
    def _json(self, body, code=200):
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _sse(self, events):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        for e in events:
            self.wfile.write(("data: " + json.dumps(e) + "\n\n").encode())
            self.wfile.flush()
            time.sleep(0.02)
        self.wfile.write(b"data: [DONE]\n\n")

    def do_POST(self):
        n = int(self.headers.get("Content-Length", 0) or 0)
        try:
            body = json.loads(self.rfile.read(n) or b"{}")
        except Exception:
            body = {}
        model = body.get("model", "mock-model")
        stream = body.get("stream", False)
        print(f"[mock:{PORT}] {self.path} model={model} stream={stream}", flush=True)
        if self.path == "/v1/chat/completions":
            if stream:
                self._sse([
                    {"id": "cmpl-1", "object": "chat.completion.chunk", "model": model,
                     "choices": [{"delta": {"role": "assistant"}, "index": 0}]},
                    {"id": "cmpl-1", "object": "chat.completion.chunk", "model": model,
                     "choices": [{"delta": {"content": "你好，这是假上游回复"}, "index": 0}]},
                    {"id": "cmpl-1", "object": "chat.completion.chunk", "model": model,
                     "choices": [{"delta": {}, "finish_reason": "stop", "index": 0}],
                     "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}},
                ])
            else:
                self._json({"id": "cmpl-1", "object": "chat.completion", "model": model,
                    "choices": [{"message": {"role": "assistant", "content": "你好，这是假上游回复"}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}})
        elif self.path == "/v1/messages":
            if stream:
                self._sse([
                    {"type": "message_start", "message": {"id": "msg-1", "type": "message", "role": "assistant",
                        "model": model, "content": [], "usage": {"input_tokens": 10, "output_tokens": 0}}},
                    {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}},
                    {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "你好，这是假上游回复"}},
                    {"type": "content_block_stop", "index": 0},
                    {"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 5}},
                ])
            else:
                self._json({"id": "msg-1", "type": "message", "role": "assistant", "model": model,
                    "content": [{"type": "text", "text": "你好，这是假上游回复"}],
                    "usage": {"input_tokens": 10, "output_tokens": 5}})
        elif self.path == "/v1/responses":
            if stream:
                self._sse([
                    {"type": "response.created", "response": {"id": "resp-1", "object": "response", "model": model,
                        "status": "in_progress", "output": []}},
                    {"type": "response.output_text.delta", "delta": "你好，这是假上游回复"},
                    {"type": "response.completed", "response": {"id": "resp-1", "object": "response", "model": model,
                        "status": "completed", "output": [{"type": "message", "role": "assistant",
                            "content": [{"type": "output_text", "text": "你好，这是假上游回复"}]}],
                        "usage": {"input_tokens": 10, "output_tokens": 5, "total_tokens": 15}}},
                ])
            else:
                self._json({"id": "resp-1", "object": "response", "model": model, "status": "completed",
                    "output": [{"type": "message", "role": "assistant",
                        "content": [{"type": "output_text", "text": "你好，这是假上游回复"}]}],
                    "usage": {"input_tokens": 10, "output_tokens": 5, "total_tokens": 15}})
        else:
            self._json({"error": "not found"}, 404)

    def log_message(self, *a):
        pass

HTTPServer(("127.0.0.1", PORT), H).serve_forever()
