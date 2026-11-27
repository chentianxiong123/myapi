#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
前后端配置字段自动对照（防漏配）。

背景（真实踩坑）：前端路由页保存 `routing.circuit_breaker.*`（嵌套对象），后端读顶层
`circuit_breaker.*`（独立 struct）——字段名相同但路径不同，serde 对未知字段静默忽略，
导致「前端改的熔断配置不生效」。此类漏配 bug 的共性：前后端字段/路径不一致且无编译期报错。

本脚本按「后端 struct ↔ 前端 interface」精确映射逐对对照：
  [WARN] 前端有、后端无            → 写了不生效（漏配 bug，需修复）
  [INFO] 后端有、前端无（非已知）   → 前端没法配（仅提示）
  [KNOWN] 映射表声明的合理差异      → 不报错（如 auto_mode 对象 vs 后端 bool）

用法：python3 tests/check_frontend_backend.py
退出码：0=无漏配；1=发现「前端有后端无」漏配。
"""

import re
import sys

ROOT = "."
FRONTEND = f"{ROOT}/web/src/lib/types.ts"
BACKEND = f"{ROOT}/rs/src/gateway.rs"

# 映射表：后端 struct ↔ 前端 interface（+ 前端嵌套路径 + 前端→后端字段别名 + 已知合理差异）
# nested_paths：前端 interface 内的嵌套对象路径（如 RoutingConfig 内联 circuit_breaker）
MAPS = [
    # (后端struct, 前端interface, 前端嵌套路径, 前端→后端别名, 已知合理差异)
    ("Channel",           "Channel",           [], {}, set()),
    ("ChannelModel",      "ChannelModel",      [], {}, set()),
    ("Provider",          "Provider",          [], {"api-key": "api_key"}, set()),
    ("RetryOverride",     "RetryOverride",     [], {}, set()),
    ("RetryConfig",       "RetryOverride",     [], {}, set()),  # 全局默认 vs 通道覆盖（子集，只查前端→后端）
    ("CircuitBreakerConfig", "RoutingConfig",  ["circuit_breaker"], {}, set()),
    ("StickyConfig",      "RoutingConfig",     ["sticky"], {}, set()),
    ("TimeoutConfig",     "RoutingConfig",     ["timeout"], {}, set()),
    ("RoutingConfig",     "RoutingConfig",     [],
     {"auto_mode": "auto_mode_enabled"},        # 前端 auto_mode 对象 vs 后端 auto_mode_enabled bool
     {"auto_mode", "capability_match"}),        # 硬约束不可关闭/仅存档，见 docs/思考模式-同识图待遇.md
]


def _brace_body(text, open_pos):
    """从 `{` 位置取匹配的 `}` 之间的体（depth 从 1 开始）。"""
    depth, i = 1, open_pos + 1
    while i < len(text) and depth > 0:
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
        i += 1
    return text[open_pos + 1:i - 1]  # 去掉外层 { }，避免 ts_fields 深度被首字符抬高


def rust_struct_fields(text, name):
    m = re.search(r"pub struct %s\b" % name, text)
    if not m:
        return set()
    brace = text.find("{", m.end())
    if brace < 0:
        return set()
    body = _brace_body(text, brace)
    return {fm.group(1) for fm in re.finditer(r"\bpub\s+(\w+)\s*:", body)}


def ts_interface_body(text, name):
    m = re.search(r"export interface %s\b" % name, text)
    if not m:
        return ""
    brace = text.find("{", m.end())
    if brace < 0:
        return ""
    return _brace_body(text, brace)


def ts_fields(body):
    """提取 interface 体内**顶层**字段名（排除注释行；嵌套对象内字段由 ts_nested_fields 单独提）。"""
    fields = set()
    depth = 0
    for line in body.splitlines():
        line = line.strip()
        if line.startswith("//") or line.startswith("/*") or line.startswith("*"):
            continue
        fm = re.match(r"(\w+)\??:", line)
        # 只认顶层（depth==0）：嵌套对象如 `circuit_breaker: {` 是顶层字段，其体内字段 depth>=1 不算
        if fm and depth == 0:
            fields.add(fm.group(1))
        depth += line.count("{") - line.count("}")
    return fields


def ts_nested_fields(body, nested_path):
    """提取 interface 内嵌套对象（如 circuit_breaker: { ... }）的字段名。"""
    pat = re.compile(r"%s\??:\s*\{" % nested_path)
    m = pat.search(body)
    if not m:
        return set()
    start = m.end()
    depth, i = 1, start
    while i < len(body) and depth > 0:
        if body[i] == "{":
            depth += 1
        elif body[i] == "}":
            depth -= 1
        i += 1
    return ts_fields(body[start:i])


def main():
    text_b = open(BACKEND, encoding="utf-8").read()
    text_f = open(FRONTEND, encoding="utf-8").read()

    warnings = []   # 前端有后端无（bug）
    infos = []      # 后端有前端无（前端没法配）
    pairs = 0

    for bstruct, finterface, nested, alias, known in MAPS:
        backend = rust_struct_fields(text_b, bstruct)
        frontend = set()
        body = ts_interface_body(text_f, finterface)
        if not body:
            print(f"[SKIP] 前端 interface {finterface} 未找到")
            continue
        if nested:
            # 嵌套映射（如 CircuitBreakerConfig ← RoutingConfig.circuit_breaker）：只用嵌套对象字段
            for np in nested:
                frontend |= ts_nested_fields(body, np)
        else:
            frontend |= ts_fields(body)
        pairs += 1

        # 前端字段 → 后端字段名（别名映射）
        feffective = set()
        for f in frontend:
            feffective.add(alias.get(f, f))

        missing = sorted(feffective - backend)   # 前端有、后端无
        extra = sorted(backend - feffective)     # 后端有、前端无

        for f in missing:
            if f in known:
                infos.append(f"[KNOWN] {bstruct}：前端 {f} 为已知合理差异（不报错）")
            else:
                warnings.append(f"{bstruct} ← {finterface}: 前端有后端无「{f}」")
        for b in extra:
            infos.append(f"{bstruct}: 后端有前端无「{b}」")

    print(f"对照 {pairs} 对结构（后端 struct ↔ 前端 interface）")
    print("=" * 60)
    if warnings:
        print(f"[WARN] 发现 {len(warnings)} 个「前端有、后端无」漏配：")
        for w in warnings:
            print(f"  - {w}")
    else:
        print("[OK] 无漏配：前端配置字段后端全部可读")
    print("-" * 60)
    for i in infos:
        print(f"[INFO] {i}")
    print("=" * 60)
    return 1 if warnings else 0


if __name__ == "__main__":
    sys.exit(main())
