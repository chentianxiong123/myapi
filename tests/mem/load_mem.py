#!/usr/bin/env python3
# 压力脚本：并发打 myapi /v1/chat/completions，统计成功/失败
import urllib.request, json, sys, time
from concurrent.futures import ThreadPoolExecutor

base = "http://127.0.0.1:17834/v1/chat/completions"

def hit(_):
    body = json.dumps({"model": "mock-model", "messages": [{"role": "user", "content": "hi"}]}).encode()
    req = urllib.request.Request(base, data=body, headers={"Content-Type": "application/json"})
    t0 = time.time()
    try:
        with urllib.request.urlopen(req, timeout=10) as r:
            return (r.status, time.time() - t0)
    except Exception:
        return (-1, time.time() - t0)

n = int(sys.argv[1]) if len(sys.argv) > 1 else 500
workers = int(sys.argv[2]) if len(sys.argv) > 2 else 50
t0 = time.time()
with ThreadPoolExecutor(max_workers=workers) as ex:
    rs = list(ex.map(hit, range(n)))
elapsed = time.time() - t0
ok = sum(1 for s, _ in rs if s == 200)
lat = [l for s, l in rs if s == 200]
avg_lat = sum(lat) / len(lat) if lat else 0
print(f"total={n} ok={ok} fail={len(rs)-ok} elapsed={elapsed:.1f}s avg_lat={avg_lat*1000:.1f}ms")
