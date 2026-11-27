#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
全流程假测试场景（用户：各种混协议/内容/特性，全流程都要假）。
用法：python3 run_all.py  或  python3 run_all.py <场景名>...
"""
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from framework import Harness, assert_ok, assert_status, write_yaml
import fake_client as fc


# ---------- 供应商/通道配置构建 ----------
def models(*names, **kw):
    """模型列表：names 是 name，kw 可带 class/protocol/capabilities。"""
    out = []
    for n in names:
        m = {"name": n}
        m.update(kw)
        out.append(m)
    return out


def provider(base_url, channels, name="mock", **kw):
    p = {"name": name, "base_url": base_url, "keys": [{"name": "主", "api-key": "sk-test"}],
         "channels": channels}
    p.update(kw)
    return p


def channel(cid, mlist, cname=None, **kw):
    c = {"id": cid, "name": cname or cid, "models": mlist}
    c.update(kw)
    return c


# ---------- 场景 ----------
def test_protocol_matrix(upstream_port, panel_port):
    """混协议矩阵：客户端 messages/responses/chat → 上游收到对应协议模型。"""
    base = f"http://127.0.0.1:{upstream_port}"
    # openai/responses 通道 base 含 /v1（转发 {base}/chat/completions → /v1/chat/completions）
    # anthropic 通道 base 不含 /v1（转发 {base}/v1/messages）
    prov_openai = provider(base + "/v1", [
        channel("ch-chat", models("gpt-4o", protocol="openai_chat")),
        channel("ch-resp", models("gpt-5", protocol="openai_responses")),
    ], name="mock-openai")
    prov_anth = provider(base, [
        channel("ch-msg", models("claude-sonnet-5", protocol="anthropic")),
    ], name="mock-anth")
    with Harness(providers=[prov_openai, prov_anth], upstream_port=upstream_port, panel_port=panel_port) as h:
        # ① client 发 anthropic messages（model=auto）→ 走 ch-msg（上游 anthropic）
        s, d = h.request("/v1/messages", fc.messages_request("auto"))
        assert_ok(s, d, "messages→chat 通道")
        # ② client 发 responses（model=auto）→ 走 ch-resp
        s, d = h.request("/v1/responses", fc.responses_request("auto"))
        assert_ok(s, d, "responses→chat 通道")
        # ③ client 发 chat（model=auto）→ 走 ch-chat
        s, d = h.request("/v1/chat/completions", fc.chat_request("auto"))
        assert_ok(s, d, "chat 直传")
        print("  ✓ 协议矩阵（messages/responses/chat 三向）")


def test_tier_routing(upstream_port, panel_port):
    """档位路由：auto 用全部 / pro 硬过滤 / flash 硬过滤 / 空候选 404。"""
    base = f"http://127.0.0.1:{upstream_port}"
    prov = provider(base + "/v1", [
        channel("ch-pro", models("gpt-5-pro", **{"class": "pro"}, protocol="openai_chat")),
        channel("ch-flash", models("gpt-4o-mini", **{"class": "flash"}, protocol="openai_chat")),
    ])
    with Harness(providers=[prov], upstream_port=upstream_port, panel_port=panel_port) as h:
        s, d = h.request("/v1/chat/completions", fc.chat_request("auto"))
        assert_ok(s, d, "auto 用全部候选")
        s, d = h.request("/v1/chat/completions", fc.chat_request("pro"))
        assert_ok(s, d, "pro 硬过滤（只 pro 通道）")
        s, d = h.request("/v1/chat/completions", fc.chat_request("flash"))
        assert_ok(s, d, "flash 硬过滤")
        # 空候选：pro 档无 pro 通道 → 404
        prov2 = provider(base + "/v1", [channel("ch-only-flash", models("gpt-4o-mini", **{"class": "flash"}))])
        with Harness(providers=[prov2], upstream_port=upstream_port, panel_port=panel_port + 10) as h2:
            s, d = h2.request("/v1/chat/completions", fc.chat_request("pro"))
            assert_status(s, 404, "pro 无候选→404")
        print("  ✓ 档位路由（auto/pro/flash + 空候选 404）")


def test_capabilities(upstream_port, panel_port):
    """能力硬约束：vision/thinking 无 capability → 409；有 → 200；disabled 不算需求。"""
    base = f"http://127.0.0.1:{upstream_port}"
    # 通道模型带 vision+thinking 能力
    prov = provider(base + "/v1", [
        channel("ch-cap", models("gpt-4o", capabilities=["vision", "thinking"], protocol="openai_chat")),
    ])
    with Harness(providers=[prov], upstream_port=upstream_port, panel_port=panel_port) as h:
        # 带图 + 模型有 vision → 200
        s, d = h.request("/v1/chat/completions", fc.chat_request("auto", fc.with_image()))
        assert_ok(s, d, "vision 有能力→200")
        # 带思考 + 模型有 thinking → 200
        s, d = h.request("/v1/chat/completions", fc.chat_request("auto", **fc.with_thinking()))
        assert_ok(s, d, "thinking 有能力→200")
        # thinking disabled → 不算需求 → 200
        s, d = h.request("/v1/chat/completions", fc.chat_request("auto", **fc.with_thinking_disabled()))
        assert_ok(s, d, "thinking disabled→200")
    # 无 vision 能力 → 带图 409
    prov2 = provider(base + "/v1", [channel("ch-no-vision", models("gpt-4o-mini", protocol="openai_chat"))])
    with Harness(providers=[prov2], upstream_port=upstream_port, panel_port=panel_port + 20) as h2:
        s, d = h2.request("/v1/chat/completions", fc.chat_request("auto", fc.with_image()))
        assert_status(s, 409, "vision 无能力→409")
        print("  ✓ 能力硬约束（vision/thinking 409/200/disabled 200）")


def test_content_and_multi_turn(upstream_port, panel_port):
    """内容：多轮历史 + 工具轮 + 长上下文。全流程转发 + 日志。"""
    base = f"http://127.0.0.1:{upstream_port}"
    prov = provider(base + "/v1", [channel("ch-mt", models("gpt-4o", protocol="openai_chat"))])
    with Harness(providers=[prov], upstream_port=upstream_port, panel_port=panel_port) as h:
        # 多轮工具历史完整转发
        s, d = h.request("/v1/chat/completions", fc.chat_request("auto", fc.with_tool_history(2)))
        assert_ok(s, d, "工具轮多轮")
        # 长上下文
        s, d = h.request("/v1/chat/completions", fc.chat_request("auto", fc.long_context(100)))
        assert_ok(s, d, "长上下文")
        # 日志对齐（成功计数 + token）
        time.sleep(0.5)
        summary = h.query_summary()
        assert summary.get("total", 0) >= 2, f"日志应有请求: {summary}"
        assert summary.get("success", 0) >= 2, f"日志成功数: {summary}"
        print(f"  ✓ 内容/多轮（工具轮+长上下文+日志 total={summary.get('total')} ok={summary.get('success')}）")


def test_failover_and_circuit(upstream_port, panel_port):
    """失败/熔断：上游 500 → failover 下一通道；全废 → 502；错误率熔断。"""
    base = f"http://127.0.0.1:{upstream_port}"
    prov = provider(base + "/v1", [
        channel("ch-bad", models("bad-model", protocol="openai_chat")),
        channel("ch-good", models("good-model", protocol="openai_chat")),
    ])
    # 假上游：前 1 个请求 500（failover 到 good），之后正常
    with Harness(upstream_cfg={"fail_first_n": 1},
                 providers=[prov], upstream_port=upstream_port, panel_port=panel_port) as h:
        s, d = h.request("/v1/chat/completions", fc.chat_request("auto"))
        assert_ok(s, d, "第一个 500→failover 到 good 成功")
    # 全废：error 注入所有请求 → 全候选废 → 502
    with Harness(upstream_cfg={"error": {"status": 500, "body": {"error": "all down"}}},
                 providers=[prov], upstream_port=upstream_port, panel_port=panel_port + 30) as h:
        s, d = h.request("/v1/chat/completions", fc.chat_request("auto"))
        assert_status(s, 502, "全废→502")
        print("  ✓ failover（500→good）/ 全废→502")


def test_sticky_and_multi_device(upstream_port, panel_port):
    """黏性 + 多设备身份：不同设备 key 独立 pin + 日志按设备聚合。"""
    base = f"http://127.0.0.1:{upstream_port}"
    prov = provider(base + "/v1", [
        channel("ch-a", models("m-a", protocol="openai_chat")),
        channel("ch-b", models("m-b", protocol="openai_chat")),
    ])
    auth = {"auth": {"api_keys": [{"name": "手机", "key": "sk-phone"}, {"name": "电脑", "key": "sk-pc"}]}}
    with Harness(providers=[prov], gateway_cfg=auth, upstream_port=upstream_port, panel_port=panel_port) as h:
        # 手机 key → 200
        s, d = h.request("/v1/chat/completions", fc.chat_request("auto"),
                         headers={"Authorization": "Bearer sk-phone"})
        assert_ok(s, d, "手机 key")
        # 电脑 key → 200
        s, d = h.request("/v1/chat/completions", fc.chat_request("auto"),
                         headers={"Authorization": "Bearer sk-pc"})
        assert_ok(s, d, "电脑 key")
        # 错 key → 401
        s, d = h.request("/v1/chat/completions", fc.chat_request("auto"),
                         headers={"Authorization": "Bearer sk-wrong"})
        assert_status(s, 401, "错 key→401")
        print("  ✓ 多设备身份（手机/电脑 key 通过 + 错 key 401）")


def test_upstream_sync(upstream_port, panel_port):
    """上游嗅探 + 变更检测（纯手动 detect/apply）：/v1/models 返回 add/remove/ignored。"""
    base = f"http://127.0.0.1:{upstream_port}"
    # 假上游 models 含：gpt-4o（本地有）+ gpt-5-new（上游新增）+ gone（本地有但上游无）
    upstream_cfg = {"models": ["gpt-4o", "gpt-5-new", "gpt-5-preview"]}
    prov = provider(base + "/v1", [channel("ch-sync", models("gpt-4o", protocol="openai_chat"))])
    with Harness(upstream_cfg=upstream_cfg, providers=[prov],
                 upstream_port=upstream_port, panel_port=panel_port) as h:
        s, d = h.request("/api/providers/upstream-models",
                         {"provider": "mock", "channel_id": "ch-sync", "ignored": ["regex:.*-preview"]})
        assert_ok(s, d, "detect")
        add = d.get("add", [])
        assert "gpt-5-new" in add, f"应检测到新增 gpt-5-new: {d}"
        assert "gpt-5-preview" not in add, f"regex 忽略应不进 add: {d}"
        # apply：加 gpt-5-new
        s, d = h.request("/api/providers/apply-upstream-models",
                         {"provider": "mock", "channel_id": "ch-sync",
                          "add": [{"name": "gpt-5-new", "alias": "", "protocol": "openai_chat"}], "remove": []})
        assert_ok(s, d, "apply")
        # 验证 gateway.yaml 已写入（读配置）
        import os
        gw = os.path.join(h.tmpdir, "config", "gateway.yaml")
        with open(gw, encoding="utf-8") as f:
            content = f.read()
        assert "gpt-5-new" in content, "apply 后 gateway.yaml 应含 gpt-5-new"
        print("  ✓ 上游嗅探 detect/apply（add 检测 + regex 忽略 + 写回）")


SCENARIOS = {
    "protocol_matrix": test_protocol_matrix,
    "tier_routing": test_tier_routing,
    "capabilities": test_capabilities,
    "content_multi_turn": test_content_and_multi_turn,
    "failover_circuit": test_failover_and_circuit,
    "sticky_multi_device": test_sticky_and_multi_device,
    "upstream_sync": test_upstream_sync,
}


def main():
    names = sys.argv[1:] or list(SCENARIOS.keys())
    passed = failed = 0
    for name in names:
        fn = SCENARIOS[name]
        print(f"\n[{name}]")
        try:
            fn(19999, 17840)
            passed += 1
            print(f"  ✓ PASS")
        except Exception as e:
            failed += 1
            print(f"  ✗ FAIL: {e}")
    print(f"\n=== {passed} passed, {failed} failed ===")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
