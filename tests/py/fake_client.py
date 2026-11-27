#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
假客户端（fake client）：多协议请求生成 + 各种内容。
模拟 Claude Code / Codex / 任意客户端发请求到 myapi 网关。
"""
import json


def chat_request(model="auto", messages=None, stream=True, **kw):
    """OpenAI chat 协议（/v1/chat/completions）。"""
    m = messages if messages is not None else [{"role": "user", "content": "你好"}]
    body = {"model": model, "messages": m, "stream": stream, "max_tokens": 100}
    body.update(kw)
    return body


def messages_request(model="auto", messages=None, stream=True, **kw):
    """Anthropic Messages 协议（/v1/messages）。"""
    m = messages if messages is not None else [{"role": "user", "content": "你好"}]
    body = {"model": model, "messages": m, "stream": stream, "max_tokens": 100}
    body.update(kw)
    return body


def responses_request(model="auto", messages=None, stream=True, **kw):
    """OpenAI Responses 协议（/v1/responses）。"""
    m = messages if messages is not None else [{"role": "user", "content": [{"type": "input_text", "text": "你好"}]}]
    body = {"model": model, "input": m, "stream": stream, "max_output_tokens": 100}
    body.update(kw)
    return body


def with_image(msg="看这张图", image_url="https://x/i.png"):
    """带图消息（vision 硬约束）。"""
    return [{"role": "user", "content": [
        {"type": "text", "text": msg},
        {"type": "image_url", "image_url": {"url": image_url}},
    ]}]


def with_tool_history(rounds=2):
    """多轮工具对话历史（user→assistant(tool_call)→tool→assistant）。"""
    msgs = []
    for i in range(rounds):
        msgs.append({"role": "user", "content": f"第{i}轮：查天气"})
        msgs.append({"role": "assistant", "content": None,
                     "tool_calls": [{"id": f"call_{i}", "type": "function",
                                     "function": {"name": "get_weather", "arguments": "{\"city\":\"北京\"}"}}]})
        msgs.append({"role": "tool", "tool_call_id": f"call_{i}", "content": "晴 25℃"})
    msgs.append({"role": "user", "content": "总结天气"})
    return msgs


def with_thinking(messages=None, effort="high"):
    """带思考（reasoning_effort / thinking 对象）。"""
    m = messages if messages is not None else [{"role": "user", "content": "思考一下"}]
    return {"messages": m, "reasoning_effort": effort}


def with_thinking_anthropic(messages=None, budget=8192):
    m = messages if messages is not None else [{"role": "user", "content": "思考一下"}]
    return {"messages": m, "thinking": {"type": "enabled", "budget_tokens": budget}}


def with_thinking_disabled(messages=None):
    m = messages if messages is not None else [{"role": "user", "content": "不思考"}]
    return {"messages": m, "thinking": {"type": "disabled"}}


def multi_turn(rounds=3):
    """多轮普通对话历史。"""
    msgs = []
    for i in range(rounds):
        msgs.append({"role": "user", "content": f"第{i}轮问"})
        msgs.append({"role": "assistant", "content": f"第{i}轮答"})
    msgs.append({"role": "user", "content": "最后一问"})
    return msgs


def long_context(msgs=200):
    """长上下文（测 context 错误反馈）。"""
    out = []
    for i in range(msgs):
        out.append({"role": "user" if i % 2 == 0 else "assistant", "content": "x" * 200})
    return out
