# myapi 项目愿景

## 一句话

**个人 LLM 网关 + 配置下发器一体化**——既是独立网关（接收请求、路由、转发），也是配置下发器（把配置同步到所有 agent）。

---

## 历史演进

### 阶段 1：手动同步（痛苦）
```
没有 myapi 时：
- 改一次模型配置 → 手动登录每台电脑 → 手动改每个 agent 的配置文件
- pi、opencode、claude、codex、omp、dsh、picoclaw、crush、openclaw、hermes...
- 10+ 个工具 × N 台机器 = 噩梦
```

### 阶段 2：myapi（自动化）
```
有了 myapi：
- 中央配置（config.yaml）→ 一键下发到所有 agent
- 10 个 adapter 自动写各agent 的配置文件
- 但依赖上游 CPA（cli-proxy-api）作为"事实来源"
```

### 阶段 3：依赖上游的痛点（现在）
```
myapi 依赖 CPA 的问题：
- CPA 配置简洁但功能有限（无延迟路由、无模型级权重）
- new-api 功能多但配置复杂（关系式、要重启、映射一对一）
- 9router/freellmapi 都不满足（都是按 tier/quota 切，不是按延迟/权重）
- 想定制化（比如"deepseek 走 SenseNova 80% + pie-xian 20%"）→ 做不到
```

### 阶段 4：一体化（目标）
```
myapi 新版本：
- 既是独立网关（不依赖 CPA/new-api）
  - 接收 OpenAI 兼容请求
  - 延迟感知路由（TTFT 快的优先）
  - 模型级 provider 权重（deepseek 80% SenseNova + 20% pie-xian）
  - 透传 UA（客户端 → 网关 → 上游，中间不改）
  - 轻量统计（异步写 SQLite）
  
- 也是配置下发器（保留原有能力）
  - 中央配置（config.yaml）= 唯一事实来源
  - 10 个 adapter 下发到所有 agent
  - 多主机支持（ssh/scp）
  - 幂等 + 备份 + 原子替换
  
- 一体化
  - 网关改配置 → 自动下发到所有 agent
  - 一个工具解决"路由 + 同步"两个问题
  - 不依赖 CPA/new-api，完全自主
```

### 迫切性：功耗 + 资源（实测 2026-11-24，meilan 手机）
```
动机：meilan 手机常驻功耗 0.7W → 1W（+40%），主因 = new-api + CPA 双进程叠加

实测（meilan-phone1，2.7GB RAM 手机）：
- new-api（PID 6006）        RSS ~95MB，VSZ 2.5GB，跑满一天+，偶发子进程冲到 113% CPU
- cli-proxy-api（CPA，PID 4693）RSS ~72MB，CPU ~4%
- 两个常驻进程叠加 → 基础功耗抬高，load average 高达 10.5（异常）

myapi 价值（直接回应）：
- 单个 Rust 进程替代 new-api + CPA → 一个轻量常驻服务
- Rust + 零拷贝透传（无 GC、无 node runtime）→ RSS 目标 <10MB 级别
- 网关 + 配置下发一体 → 不用为"路由"和"同步"跑两套
→ 目标：把手机常驻功耗拉回 ~0.7W，释放内存 + 降负载
```

---

## 核心功能

### 1. 网关（Gateway）
- 接收 OpenAI 兼容请求（`/v1/chat/completions`）
- 延迟感知路由（自动测 TTFT，首字快的优先）
- 模型级 provider 权重（同一模型在多个 provider 之间加权）
- session affinity（同会话粘同一个 provider，缓存友好）
- 透传 headers（客户端 UA 原样传给上游）
- 流式转发（SSE，不改格式）
- vision 能力硬约束：当前轮带图 → 只选有 vision 能力的通道；无可用 → 409 显式失败（不静默降级）；历史轮的图 → 占位符降级（对照 9router）
- context 不足错误反馈：全模型试、全废才反馈（客户端无感）；context_keywords → 豁免 max_retries
- 轻量统计（异步写 SQLite：时间/模型/provider/TTFT/tokens/error）
- Web UI（看统计、改配置）

### 2. 下发器（Sync）
- 中央配置 = 唯一事实来源（`config.yaml`）
- 10 个 adapter（pi/opencode/claude/codex/omp/dsh/picoclaw/crush/openclaw/hermes）
- 多主机支持（ssh/scp 下发到各设备）
- 幂等 + 备份（写前备份，内容不变不写）
- 原子替换（scp 临时文件 + 远端原子替换）
- Web UI（管理配置、触发同步）

### 3. 一体化
- 网关改配置 → 自动下发到所有 agent
- 一个 YAML 管所有（provider + model + routing + sync targets）
- 一个二进制跑起来（Rust，零依赖）
- 一个 Web UI 管理（统计 + 配置 + 同步）

---

## 差异化

| 对比 | myapi（新版）| cpa | new-api | cc-switch | LiteLLM |
|---|---|---|---|---|---|
| **独立网关** | ✅ | ✅ | ✅ | ❌ | ✅ |
| **配置下发器** | ✅ | ❌ | ❌ | ⚠️（桌面应用）| ❌ |
| **延迟感知路由** | ✅ | ❌ | ❌ | ❌ | ✅ |
| **模型级权重** | ✅ | ❌ | ❌ | ❌ | ✅ |
| **声明式配置** | ✅（YAML）| ✅（YAML）| ❌（SQL）| ⚠️（JSON）| ⚠️（YAML）|
| **单二进制** | ✅（Rust）| ✅（Go）| ✅（Go）| ❌（Tauri）| ❌（Python）|
| **不依赖上游** | ✅ | — | — | ✅ | ✅ |
| **Web UI** | ✅ | ❌ | ✅ | ⚠️（桌面）| ✅ |

**独特价值**：**唯一同时做到"独立网关 + 配置下发器 + 延迟路由 + 模型级权重 + 声明式配置"的工具。**

---

## 技术栈

- **后端**：Rust + axum + tokio + reqwest + sqlx（SQLite WAL）
- **前端**：React 18 + TypeScript + Vite + Tailwind CSS + Recharts
- **部署**：单二进制（Rust 编译）+ 静态文件（React 编译）
- **多实例**：SO_REUSEPORT（内核级负载均衡，不需要 nginx）

---

## 项目结构

```
myapi/
├── rs/                    # Rust 后端（网关 + 下发器）
│   ├── src/
│   │   ├── gateway/       # 网关核心（路由/透传/统计）
│   │   ├── sync/          # 下发器（adapter/remote）
│   │   ├── api.rs         # HTTP API
│   │   └── main.rs        # 入口
│   └── Cargo.toml
├── web/                   # React 前端
│   ├── src/
│   │   ├── pages/         # 页面（Dashboard/Config/Sync/...）
│   │   ├── components/    # 组件（StatsHero/TrendChart/...）
│   │   └── lib/           # 工具（api.ts/types.ts）
│   └── package.json
├── config/                # 配置文件
│   └── cpa-config.yaml    # 中央配置（唯一事实来源）
└── VISION.md              # 本文档
```

---

## 路线图

### Phase 1：网关 MVP（当前）
- [x] React 前端骨架 + Dashboard
- [x] Config 页（Provider + 路由策略）
- [ ] Rust 后端（axum + 路由 + 透传）
- [ ] 统计日志（SQLite）—— 日志入库 schema（后端实现时遵循，字段已定）：
  - 基础：id / timestamp / model / provider / app_type / session_id
  - **使用者：key_name**（请求带哪个 key 入库——主密钥/好友名，区分"我的/好友"）
  - Token：input_tokens / output_tokens / cache_read / cache_creation / real_total_tokens
  - 延迟：ttft_ms / latency_ms / duration_s / output_speed_tps
  - 状态：status_code / error / is_stream
- [ ] 流式转发

### Phase 2：下发器回归
- [ ] 恢复 10 个 adapter（从旧代码迁移）
- [ ] 多主机支持（ssh/scp）
- [ ] Sync 页（管理 targets/hosts/instances）

### Phase 3：一体化
- [ ] 网关改配置 → 自动触发下发
- [ ] Web UI 统一管理（网关 + 下发器）
- [ ] 多实例支持（SO_REUSEPORT）

### Phase 4：优化
- [ ] 延迟感知路由（自动测 TTFT）
- [ ] 模型级 provider 权重
- [ ] 缓存/压缩（可选）
- [ ] 改名（脱离 cpa，独立品牌）

---

## 改名计划

**当前**：myapi（历史遗留，依赖 CPA）
**未来**：待定（可能叫 `llm-gateway`、`personal-router`、`agent-hub` 等）

**改名条件**：
- 网关功能稳定
- 下发器功能回归
- 不再依赖 CPA（完全独立）

---

## 设计理念

1. **一体化**：网关 + 下发器，不是二选一
2. **声明式**：一个 YAML 管所有（provider + model + routing + sync）
3. **单二进制**：Rust 编译，零依赖，扔服务器就能跑
4. **延迟优先**：自动测TTFT，首字快的优先
5. **透传**：客户端 → 网关 → 上游，中间不改（UA/headers）
6. **轻量统计**：异步写 SQLite，不影响性能
7. **普通人友好**：Web UI 管理，不用命令行

---

*最后更新：2026-11-23*
