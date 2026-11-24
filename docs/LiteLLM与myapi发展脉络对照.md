# LiteLLM 与 myapi 发展脉络对照

> 结论：**发展脉络高度重合**。LiteLLM 是我们的"未来时"——它正在走的路，正是我们要一次性走到的终点。
> 区别只在于：LiteLLM 背着 Python 历史包袱**渐进迁移**，我们**从零全 Rust** 直接落到终点。

---

## 一、并存的两个入口（最像的地方）

LiteLLM 现在**同时有两个网关入口**：

| 入口 | 语言 | 位置 | 状态 |
|---|---|---|---|
| 老 proxy | Python | `litellm/proxy/proxy_server.py` | 历史主体，功能全 |
| 新网关 | **Rust (axum)** | `litellm/rust/crates/gateway/src/main.rs` | 新核心，`axum::serve` 起服务 |

Rust 网关**独立可跑**（自带 axum HTTP server），正在从 Python 手里接管热路径。

**这正是我们要做的事**：用一个 Rust 进程替代现有方案。

---

## 二、LiteLLM 的迁移动机（逐条对应我们的痛点）

### ADR 001：Only type requests inside Rust
- **痛点**：Python SDK 类型浅，本质是 untyped kwargs 壳，多次入口路径让类型形同虚设
- **决策**：不在 Python 里补类型，**把原始请求传进 Rust**，在边界做转换
- **我们**：从零就 Rust 强类型（struct + serde），没这包袱

### 为什么把路由核心迁 Rust（性能）
- 路由/请求处理是**热路径**，Python 慢 + 内存高
- LiteLLM 把 `router-core`、`gateway`、`gateway-inference` 全用 Rust 重写
- **我们**：目标就是**降功耗**（meilan 0.7W→1W，new-api+CPA 双进程 167MB）
  → Rust 单进程低内存，动机完全一致，只是我们更彻底

---

## 三、发展脉络对照表

| 阶段 | LiteLLM | myapi |
|---|---|---|
| **起点** | Python SDK → 加 proxy（Python）| 依赖 CPA（cli-proxy-api）+ new-api 双进程 |
| **痛点 1** | Python 类型浅、路由慢 | CPA 功能有限、new-api 复杂、双进程高功耗 |
| **痛点 2** | 想定制路由（延迟/权重）做不到 | 想定制（deepseek 80/20）做不到 |
| **动作** | 把路由核心 + 网关**用 Rust 重写** | **从零写 Rust 网关**（axum） |
| **架构** | Python 主体 + Rust 网关（混合，渐进迁移）| **全 Rust**（一步到位，无历史包袱）|
| **路由核心** | `router-core`（Rust）：deployment、cooldown、权重、延迟感知 | 照抄这套设计 |
| **deployment/通道** | Deployment（key×base×映射绑死）| 通道 Channel（key×路径×映射绑死）|
| **熔断** | 被动 cooldown（失败率>50%/429）| 被动 cooldown（同款）|
| **网关框架** | axum | axum |
| **终极目标** | 替代老 Python proxy | 替代 new-api + CPA 双进程 |

---

## 四、我们比 LiteLLM 更激进的地方

| 维度 | LiteLLM | myapi |
|---|---|---|
| **历史包袱** | 有（Python SDK + 老 proxy 要兼容）| 无（从零）|
| **迁移策略** | 渐进（Python/Rust 双入口并存，逐步接管）| 一步到位（直接全 Rust）|
| **附加能力** | 纯网关 | **网关 + 配置下发一体**（10 adapter 下发到 agent）|
| **成本记账** | 有 cost 追踪 | **不要成本**（自用）|
| **定位** | 通用 LLM 网关（企业级，多租户）| 个人网关（自用，单用户）|

**关键差异**：LiteLLM 是**企业级通用网关**（多租户、成本、Redis 分布式），我们**个人网关**（单用户、轻量、降功耗、还要配置下发）。所以我们只抄它的**路由核心**，不要它的企业级复杂度。

---

## 五、抄什么 / 不抄什么

### ✅ 抄（纯 Rust，直接参考）
- `router-core/`：deployment 注册、cooldown、权重、延迟感知、健康检查
- `gateway/`：axum 网关骨架、请求入口
- `gateway-inference/`：请求处理、流式、headers

### ❌ 不抄
- Python 部分（`litellm/proxy/`）——历史包袱，正是我们要摆脱的
- 企业级复杂度：多租户、cost 记账、Redis 分布式（我们单用户自用）
- 成本相关（我们要 cost=0 永久归零）

---

## 六、一句话总结

> **LiteLLM 走了"Python 起家 → 痛点 → Rust 重写网关核心"的路，我们现在正站在它"Rust 重写"那一步——但我们没有 Python 包袱，直接全 Rust 一步到位。**
> 它的 `router-core`（Rust）就是我们后端的路由蓝图；它的混合架构是我们该避免的中间态。

---

## 附：源码位置（浅克隆 `/mnt/shared/ref-project/api/litellm/`）

- Rust 工作区：`litellm/rust/Cargo.toml`（workspace，30+ crates）
- 路由核心：`litellm/rust/crates/router-core/src/`（strategy/weights/health/health_routing）
- 网关入口：`litellm/rust/crates/gateway/src/main.rs`（axum serve）
- 请求处理：`litellm/rust/crates/gateway-inference/src/`
- 架构决策：`litellm/rust/docs/adrs/adr_001_only_type_inference_inside_rust.md`
- 详细路由设计：见 `docs/LiteLLM路由核心研究.md`
