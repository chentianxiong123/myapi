# 网关内存实测脚本（Windows 11）

2026-11-26 实测 myapi 一体化网关内存（空载/压力/增长率），脚本从 win11 拷回存档。
完整数据与结论见 `docs/性能对比.md`「网关空载/压力内存实测」章节。

## 脚本清单

| 脚本 | 用途 |
|------|------|
| `mock_upstream.py` | 假供应商 mock 上游（stdlib，无依赖），参数=端口（实测用 9999）|
| `load_mem.py` | 并发打 `POST /v1/chat/completions`，输出 total/ok/fail/elapsed/avg_lat |
| `mem_full.ps1` | **完整生命周期**：清理→启动 mock+myapi→记 T0→压 500→记 T1→空闲 30s→记 T2→再压 500→记 T3 |
| `mem_press.ps1` | 简化版：启动+记 WS0→压 500→记 WS1（+delta/growth_pct）|
| `mem_press2.ps1` | 泄漏判据：空闲 30s 后读→再压 500 读（看两轮是否稳定）|

## 使用（win11）

```powershell
# 完整生命周期曲线（T0/T1/T2/T3）
ssh win11 "powershell -NoProfile -ExecutionPolicy Bypass -File C:\Users\a1\myapi\tests\mem_full.ps1"
```

> ⚠️ **坑**：win11 SSH 断开会回收子进程，**必须单条 SSH 会话内一次跑完**（脚本内已含全部阶段）。
> 依赖：`rs/config/gateway.yaml` 的 provider base_url 指向 mock 上游端口（实测 9999），
> myapi 二进制在 `rs/target/debug/myapi.exe`。

## 实测结论摘要（2026-11-26）

- 空载 **10.16 MB**（release 早前测 8.5 MB）
- 压力峰值 **20.80 MB**（50 并发 500 请求）
- **两轮压力间 +3.7% → 水位稳定，无内存泄漏**
- 对照：new-api 28MB、LiteLLM 503MB（轻约 60 倍）
