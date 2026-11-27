#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
可配置假上游（fake upstream）：多协议 + 各种内容/特性/错误注入。
用户要求：假上游、假客户端一切都要假，全流程，各种混协议/内容/特性。

用法：python3 fake_upstream.py <port> [config.json]
  config.json 控制行为（可缺省=全默认无错误）。

行为配置（全部可选）：
{
  "models": ["gpt-4o", "gpt-4o-mini", "claude-sonnet-5", "gemini-2.0-flash", "llama3:latest"],
  "model_reply": "你好，这是假上游回复",
  "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15},
  "delay_ms": 0,                     # 每请求延迟（测 TTFT/超时）
  "error": {"status": 500, "body": {"error": "upstream down"}},   # 注入 HTTP 错误（每请求）
  "fail_first_n": 0,                 # 前 N 个请求返回 error（测 failover）
  "tool_calls": true,                # 响应带工具调用（第一轮）
  "thinking": true,                  # 响应带思考（reasoning_content / thinking block）
  "keywords": {"欠费": 402, "封号": 403},   # 关键词→状态码（欠费/封号/配额）
  "context": {"max_tokens": 1000},   # 超长上下文→400 context_length_exceeded
  "sse_ttft_ms": 0                   # 流式首字延迟（测 streaming_first_byte 超时）
}
"""
import json
import sys
import time
from http.server import HTTPServer, BaseHTTPRequestHandler


class FakeUpstream:
    def __init__(self, cfg):
        self.cfg = cfg
        self.calls = []          # 记录每个请求（断言用）：{path, body, model, headers}
        self.count = 0

    def record(self, path, body, headers):
        self.count += 1
        self.calls.append({"path": path, "body": body, "model": body.get("model"),
                           "headers": dict(headers)})
        return self.count

    def should_error(self):
        """错误注入：error 配置 或 fail_first_n 前 N 次。"""
        if self.cfg.get("error"):
            return True
        n = self.cfg.get("fail_first_n", 0)
        if n > 0 and self.count <= n:
            return True
        return False


def build_handler(fu):
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

        def _read_body(self):
            n = int(self.headers.get("Content-Length", 0) or 0)
            try:
                return json.loads(self.rfile.read(n) or b"{}")
            except Exception:
                return {}

        def _maybe_error(self):
            """关键词熔断（欠费/封号）在错误返回前检查；error 配置注入。"""
            if not fu.should_error():
                return None
            err = fu.cfg.get("error") or {"status": 500, "body": {"error": "upstream fail"}}
            self._json(err.get("body", {}), err.get("status", 500))
            return True

        def do_GET(self):
            cfg = fu.cfg
            if self.path == "/__calls":
                # 测试框架读取假上游收到的请求记录（断言协议/模型/头）
                self._json({"calls": fu.calls, "count": fu.count})
                return
            models = cfg.get("models", ["gpt-4o", "gpt-4o-mini"])
            if self.path.startswith("/v1beta/models"):
                # Gemini 形：{models:[{name:"models/xxx"}]}
                self._json({"models": [{"name": "models/" + m} for m in models]})
            elif self.path.startswith("/api/tags"):
                # Ollama 形：{models:[{name:"xxx:latest"}]}
                self._json({"models": [{"name": m} for m in models]})
            elif self.path.startswith("/v1/models") or self.path.startswith("/models"):
                # OpenAI/Anthropic 形：{data:[{id}]}
                self._json({"data": [{"id": m} for m in models]})
            else:
                self._json({"error": "not found"}, 404)

        def do_POST(self):
            if self.path == "/__recover":
                fu.cfg["fail_first_n"] = 0
                self._json({"ok": True})
                return
            body = self._read_body()
            model = body.get("model", "mock-model")
            stream = body.get("stream", False)
            fu.record(self.path, body, self.headers)
            delay = fu.cfg.get("delay_ms", 0)
            if delay:
                time.sleep(delay / 1000.0)
            if self._maybe_error():
                return
            # 关键词熔断（在正常返回前检查关键词，模拟 new-api 欠费/封号）
            if fu.cfg.get("keywords"):
                for kw, code in fu.cfg["keywords"].items():
                    if any(kw in str(m.get("content", "")) for m in body.get("messages", [])
                           if isinstance(m, dict)):
                        self._json({"error": {"type": "insufficient_quota" if code in (402, 403) else "error",
                                              "message": f"{kw}（上游关键词命中）"}}, code)
                        return
            reply = fu.cfg.get("model_reply", "你好，这是假上游回复")
            usage = fu.cfg.get("usage", {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15})
            # 超长上下文 → 400 context（学 new-api / LiteLLM）
            ctx = fu.cfg.get("context")
            if ctx and body.get("max_tokens"):
                # 按模型限制（model_limits[model] 或默认 max_tokens）；模拟"某通道装不下、某通道装得下"
                limit = ctx.get("model_limits", {}).get(model, ctx.get("max_tokens", 1000))
                if body["max_tokens"] > limit:
                    self._json({"error": {"type": "context_length_exceeded", "message": "超长上下文"}}, 400)
                    return
            print(f"[fake:{PORT}] {self.path} model={model} stream={stream} "
                  f"msgs={len(body.get('messages', []))}", flush=True)

            if self.path == "/v1/chat/completions":
                self._chat(body, model, stream, reply, usage)
            elif self.path == "/v1/messages":
                self._messages(body, model, stream, reply, usage)
            elif self.path == "/v1/responses":
                self._responses(body, model, stream, reply, usage)
            else:
                self._json({"error": "not found"}, 404)

        def _chat(self, body, model, stream, reply, usage):
            cfg = fu.cfg
            if stream:
                events = [
                    {"id": "cmpl-1", "object": "chat.completion.chunk", "model": model,
                     "choices": [{"delta": {"role": "assistant"}, "index": 0}]},
                ]
                if cfg.get("thinking"):
                    events.append({"id": "cmpl-1", "object": "chat.completion.chunk", "model": model,
                                   "choices": [{"delta": {"reasoning_content": "思考中…"}, "index": 0}]})
                if cfg.get("tool_calls"):
                    events.append({"id": "cmpl-1", "object": "chat.completion.chunk", "model": model,
                                   "choices": [{"delta": {"tool_calls": [{"index": 0, "id": "call_1",
                                       "type": "function", "function": {"name": "get_weather", "arguments": "{\"city\":\"北京\"}"}}]}, "index": 0}]})
                else:
                    events.append({"id": "cmpl-1", "object": "chat.completion.chunk", "model": model,
                                   "choices": [{"delta": {"content": reply}, "index": 0}]})
                events.append({"id": "cmpl-1", "object": "chat.completion.chunk", "model": model,
                               "choices": [{"delta": {}, "finish_reason": "stop", "index": 0}], "usage": usage})
                self._sse(events)
            else:
                msg = {"role": "assistant", "content": reply}
                if cfg.get("thinking"):
                    msg["reasoning_content"] = "思考中…"
                if cfg.get("tool_calls"):
                    msg["tool_calls"] = [{"id": "call_1", "type": "function",
                                          "function": {"name": "get_weather", "arguments": "{\"city\":\"北京\"}"}}]
                self._json({"id": "cmpl-1", "object": "chat.completion", "model": model,
                            "choices": [{"message": msg, "finish_reason": "stop"}], "usage": usage})

        def _messages(self, body, model, stream, reply, usage):
            cfg = fu.cfg
            if stream:
                events = [
                    {"type": "message_start", "message": {"id": "msg-1", "type": "message",
                        "role": "assistant", "model": model, "content": [],
                        "usage": {"input_tokens": usage.get("prompt_tokens", 10), "output_tokens": 0}}},
                ]
                if cfg.get("thinking"):
                    events.append({"type": "content_block_start", "index": 0,
                                   "content_block": {"type": "thinking", "thinking": "思考中…"}})
                    events.append({"type": "content_block_delta", "index": 0,
                                   "delta": {"type": "thinking_delta", "thinking": "思考中…"}})
                    events.append({"type": "content_block_stop", "index": 0})
                events.append({"type": "content_block_start", "index": 1,
                               "content_block": {"type": "text", "text": ""}})
                events.append({"type": "content_block_delta", "index": 1,
                               "delta": {"type": "text_delta", "text": reply}})
                events.append({"type": "content_block_stop", "index": 1})
                events.append({"type": "message_delta", "delta": {"stop_reason": "end_turn"},
                               "usage": {"output_tokens": usage.get("completion_tokens", 5)}})
                self._sse(events)
            else:
                content = []
                if cfg.get("thinking"):
                    content.append({"type": "thinking", "thinking": "思考中…"})
                content.append({"type": "text", "text": reply})
                self._json({"id": "msg-1", "type": "message", "role": "assistant", "model": model,
                            "content": content,
                            "usage": {"input_tokens": usage.get("prompt_tokens", 10),
                                      "output_tokens": usage.get("completion_tokens", 5)}})

        def _responses(self, body, model, stream, reply, usage):
            cfg = fu.cfg
            if stream:
                events = [
                    {"type": "response.created", "response": {"id": "resp-1", "object": "response",
                        "model": model, "status": "in_progress", "output": []}},
                ]
                if cfg.get("thinking"):
                    events.append({"type": "response.output_text.delta", "delta": "思考中…"})
                events.append({"type": "response.output_text.delta", "delta": reply})
                events.append({"type": "response.completed", "response": {"id": "resp-1",
                    "object": "response", "model": model, "status": "completed",
                    "output": [{"type": "message", "role": "assistant",
                        "content": [{"type": "output_text", "text": reply}]}], "usage": usage}})
                self._sse(events)
            else:
                self._json({"id": "resp-1", "object": "response", "model": model, "status": "completed",
                            "output": [{"type": "message", "role": "assistant",
                                "content": [{"type": "output_text", "text": reply}]}], "usage": usage})

        def log_message(self, *a):
            pass

    return H


if __name__ == "__main__":
    PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 19999
    cfg = {}
    if len(sys.argv) > 2:
        with open(sys.argv[2]) as f:
            cfg = json.load(f)
    fu = FakeUpstream(cfg)
    HTTPServer(("127.0.0.1", PORT), build_handler(fu)).serve_forever()
