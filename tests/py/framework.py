#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
假测试框架（Python）：起假上游 + 起 myapi 网关 + 全流程断言。
用户要求：假上游/假客户端一切都要假，全流程，各种混协议/内容/特性。

依赖：myapi 二进制（rs/target/debug/myapi，本机实测编译）+ Python3 + requests。
隔离：MYAPI_ROOT=<tempdir>，网关读 tempdir/config/gateway.yaml，不碰真实 rs/config/。

用法：
    from framework import Harness
    with Harness(upstream_cfg={...}, providers=[...]) as h:
        h.request(...)
"""
import json
import os
import shutil
import sqlite3
import subprocess
import tempfile
import time
import urllib.request
import urllib.error

import requests

MYAPI_BIN = os.environ.get(
    "MYAPI_BIN",
    os.path.join(os.path.dirname(__file__), "..", "..", "rs", "target", "debug", "myapi"),
)
PY_DIR = os.path.dirname(os.path.abspath(__file__))
FAKE_UPSTREAM = os.path.join(PY_DIR, "fake_upstream.py")

DEFAULT_UPSTREAM_PORT = 19999
DEFAULT_PANEL_PORT = 17840


def write_yaml(path, data):
    """dict → yaml（用 yaml 库或手写；优先 PyYAML）。"""
    try:
        import yaml
        with open(path, "w", encoding="utf-8") as f:
            yaml.safe_dump(data, f, allow_unicode=True, sort_keys=False)
    except ImportError:
        # 兜底：手写简化 yaml（仅测试配置用）
        def dump(v, indent=0):
            sp = "  " * indent
            lines = []
            if isinstance(v, dict):
                for k, val in v.items():
                    if isinstance(val, (dict, list)):
                        lines.append(f"{sp}{k}:")
                        lines.append(dump(val, indent + 1))
                    elif isinstance(val, bool):
                        lines.append(f"{sp}{k}: {'true' if val else 'false'}")
                    elif isinstance(val, (int, float)):
                        lines.append(f"{sp}{k}: {val}")
                    else:
                        lines.append(f"{sp}{k}: \"{str(val).replace(chr(34), chr(39))}\"")
            elif isinstance(v, list):
                for item in v:
                    if isinstance(item, dict):
                        lines.append(f"{sp}-")
                        lines.append(dump(item, indent + 1))
                    else:
                        lines.append(f"{sp}- \"{item}\"")
            return "\n".join(lines)
        with open(path, "w", encoding="utf-8") as f:
            f.write(dump(data) + "\n")


class Harness:
    """起假上游 + 起网关，跑场景，收尾清理。"""

    def __init__(self, upstream_cfg=None, providers=None, gateway_cfg=None,
                 upstream_port=None, panel_port=None):
        self.upstream_port = upstream_port or DEFAULT_UPSTREAM_PORT
        self.panel_port = panel_port or DEFAULT_PANEL_PORT
        self.upstream_cfg = upstream_cfg or {}
        self.providers = providers or []
        self.gateway_cfg = gateway_cfg or {}
        self.tmpdir = None
        self.upstream_proc = None
        self.gateway_proc = None
        self.base_url = f"http://127.0.0.1:{self.panel_port}"
        self.upstream_url = f"http://127.0.0.1:{self.upstream_port}"

    # ---- 生命周期 ----
    def __enter__(self):
        self.tmpdir = tempfile.mkdtemp(prefix="myapi-test-")
        os.makedirs(os.path.join(self.tmpdir, "config"), exist_ok=True)
        # 假上游配置
        cfg_path = os.path.join(self.tmpdir, "upstream_cfg.json")
        with open(cfg_path, "w", encoding="utf-8") as f:
            json.dump(self.upstream_cfg, f)
        self.upstream_proc = subprocess.Popen(
            ["python3", FAKE_UPSTREAM, str(self.upstream_port), cfg_path],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        # 网关配置
        # gateway_cfg 的段（auth 等）放顶层；listen/panel_port 固定进 gateway 段
        gw = {"gateway": {"listen": 8380, "panel_port": self.panel_port}, "providers": self.providers}
        gw.update(self.gateway_cfg)
        write_yaml(os.path.join(self.tmpdir, "config", "gateway.yaml"), gw)
        # 起网关（MYAPI_ROOT 隔离）
        env = dict(os.environ)
        env["MYAPI_ROOT"] = self.tmpdir
        self.gateway_proc = subprocess.Popen(
            [MYAPI_BIN], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        self._wait_ready(self.upstream_url + "/v1/models", is_upstream=True)
        self._wait_ready(self.base_url + "/api/config", is_upstream=False)
        return self

    def __exit__(self, *a):
        for p in (self.gateway_proc, self.upstream_proc):
            if p:
                p.terminate()
                try:
                    p.wait(timeout=5)
                except Exception:
                    p.kill()
        if self.tmpdir:
            shutil.rmtree(self.tmpdir, ignore_errors=True)

    def _wait_ready(self, url, is_upstream, timeout=15):
        for _ in range(timeout * 10):
            try:
                r = requests.get(url, timeout=0.5)
                if r.status_code < 500:
                    return True
            except Exception:
                pass
            time.sleep(0.1)
        raise RuntimeError(f"服务未就绪: {url}")

    # ---- 请求 ----
    def request(self, path, body=None, headers=None, method="POST", timeout=30):
        h = dict(headers or {})
        h.setdefault("Content-Type", "application/json")
        if method == "POST":
            r = requests.post(self.base_url + path, json=body, headers=h, timeout=timeout)
        elif method == "GET":
            r = requests.get(self.base_url + path, headers=h, timeout=timeout)
        else:
            r = requests.request(method, self.base_url + path, json=body, headers=h, timeout=timeout)
        try:
            return r.status_code, r.json()
        except Exception:
            return r.status_code, {"raw": r.text}

    # ---- 假上游调用记录 ----
    def upstream_calls(self):
        """读假上游收到的请求记录（/__calls 端点）：断言上游收到什么协议/模型/头。"""
        try:
            r = requests.get(self.upstream_url + "/__calls", timeout=3)
            return r.json().get("calls", [])
        except Exception:
            return []

    # ---- SQLite 日志 ----
    def query_logs(self, limit=20):
        db = os.path.join(self.tmpdir, "config", "gateway-logs.db")
        if not os.path.exists(db):
            return []
        conn = sqlite3.connect(db)
        conn.row_factory = sqlite3.Row
        try:
            cur = conn.execute(
                "SELECT * FROM request_logs ORDER BY ts DESC LIMIT ?", (limit,))
            return [dict(r) for r in cur.fetchall()]
        finally:
            conn.close()

    def query_summary(self):
        db = os.path.join(self.tmpdir, "config", "gateway-logs.db")
        if not os.path.exists(db):
            return {}
        conn = sqlite3.connect(db)
        conn.row_factory = sqlite3.Row
        try:
            cur = conn.execute("""
                SELECT COUNT(*) total,
                       SUM(CASE WHEN status < 400 THEN 1 ELSE 0 END) success,
                       SUM(CASE WHEN status >= 400 THEN 1 ELSE 0 END) error,
                       COALESCE(SUM(input_tokens),0) input_tokens,
                       COALESCE(SUM(output_tokens),0) output_tokens
                FROM request_logs
            """)
            return dict(cur.fetchone())
        finally:
            conn.close()


# ---- 常用断言 ----
def assert_ok(status, data, label="请求"):
    assert status == 200, f"{label}: 期望 200 实际 {status}，body={data}"


def assert_status(status, want, label="请求"):
    assert status == want, f"{label}: 期望 {want} 实际 {status}"
