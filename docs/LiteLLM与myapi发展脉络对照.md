# LiteLLM 与 myapi 发展脉络对照

> 结论：**发展脉络高度重合，且起点同源**。两者都从"**配置管理**"起家，后来因为痛点才长出"**网关/路由**"。
> LiteLLM 是我们的"未来时"——它正在走的演化路，正是我们要走的。区别：它背 Python 包袱渐进迁移，我们从零全 Rust 直达终点。

---

## 一、起点同源：都是从"配置管理"开始

| | LiteLLM 起点 | myapi 起点 |
|---|---|---|
| **根源** | Python SDK——统一调用各家 LLM（配置/封装各家 API 差异）| 配置下发器——中央 config.yaml 一键下发到 10+ agent |
| **本质** | **配置管理层**（把复杂的 LLM 调用配置化、标准化）| **配置管理层**（把分散的 agent 配置集中化、自动化）|
| **最初不涉及** | 路由、网关、冷却 | 路由、网关、冷却 |

**核心共鸣**：两者起点都是"**把乱七八糟的配置管起来**"，不是一上来就做网关。
- LiteLLM：先做统一调用 SDK（管理"怎么调各家模型"的配置）
- myapi：先做配置下发（管理"每个 agent 用什么配置"）

---

## 二、myapi 的真实发展阶段（VISION.md）

```
阶段 1：手动同步（痛苦）
  改一次模型配置 → 手动登录每台电脑 → 手动改每个 agent 配置文件
  pi/opencode/claude/codex/omp/dsh/picclaw/crush/openclaw/hermes... 10+ 工具 × N 台机器 = 噩梦

阶段 2：myapi 配置下发器（自动化）
  中央 config.yaml → 一键下发 → 10 个 adapter 自动写各 agent 配置
  但依赖上游 CPA（cli-proxy-api）作为"事实来源"（模型/路由配置在 CPA）

阶段 3：依赖上游的痛点（现在）
  CPA 配置简洁但功能有限（无延迟路由、无模型级权重）
  → 想定制（deepseek 走 SenseNova 80% + pie-xian 20%）做不到
  → 于是决定：myapi 自己长出网关能力（路由），不再只当下发器

阶段 4：一体化（目标）
  网关（路由/转发/统计）+ 配置下发器，一个 Rust 进程，替代 CPA + new-api
```

**关键**：CPA / new-api 是**阶段 2-3 的依赖和痛点**，不是起点。**起点是"配置同步的痛苦"**。

---

## 三、LiteLLM 的对应发展阶段

```
阶段 1：Python SDK（起点）
  统一调用各家 LLM（OpenAI/Anthropic/...），封装 API 差异
  → 本质是"配置/封装管理层"

阶段 2：加 proxy（网关雏形）
  加了 Python proxy server，能集中配置、管理 key
  → 从"SDK 库"长出"网关"

阶段 3：痛点
  Python 类型浅、路由慢、难定制（延迟感知/权重路由做不到）
  → 想定制路由策略做不到

阶段 4：Rust 重写网关核心（现在）
  router-core（Rust）：deployment、cooldown、权重、延迟感知
  gateway（Rust axum）：新入口
  → 渐进迁移：Python proxy + Rust gateway 并存
```

---

## 四、脉络逐段对照（修正版）

| 阶段 | LiteLLM | myapi |
|---|---|---|
| **起点（同源）** | Python SDK：统一调用配置 | 配置下发器：统一 agent 配置 |
| **本质** | 配置管理层 | 配置管理层 |
| **长出网关** | 加 Python proxy（集中配置/管 key）| 加路由能力（不再只下发，自己做路由）|
| **痛点** | Python 路由慢、类型浅、难定制延迟/权重 | 依赖 CPA、无延迟路由/权重、双进程高功耗 |
| **动作** | 路由核心 + 网关 **Rust 重写** | **从零 Rust 网关** |
| **架构现在** | Python proxy + Rust gateway 并存（渐进迁移）| 目标：全 Rust 单进程 |
| **deployment/通道** | Deployment（key×base×映射绑死）| 通道 Channel（key×路径×映射绑死）|
| **熔断** | 被动 cooldown | 被动 cooldown |
| **终极目标** | 替代老 Python proxy | 替代 CPA + new-api 依赖 |

---

## 五、共鸣点：都是"配置管理 → 被逼长出网关"

**最深的共鸣**：两者都不是一开始就想做网关，都是**因为配置管理的痛点，被逼着长出路由/网关能力**：

- **LiteLLM**：统一调用配置 → 用户要在多处用不同模型/key/路由 → 被逼做 proxy → proxy 慢/难定制 → 被逼用 Rust 重写路由
- **myapi**：统一 agent 配置 → 依赖 CPA 当事实来源 → CPA 做不到延迟路由/权重 → 被逼自己做网关 → 决定全 Rust

**所以演进逻辑一模一样**：配置管理 → 集中化 → 需要路由 → 需要高性能路由 → Rust。

---

## 六、我们比 LiteLLM 更激进 / 更聚焦

| 维度 | LiteLLM | myapi |
|---|---|---|
| **历史包袱** | 有（Python SDK + 老 proxy 要兼容）| 无（从零）|
| **迁移** | 渐进（双入口并存）| 一步到位（全 Rust）|
| **附加** | 纯网关 | **网关 + 配置下发一体**（保留初心能力）|
| **成本** | cost 追踪 | 不要成本（自用）|
| **定位** | 企业级通用网关（多租户/Redis）| 个人网关（单用户/轻量/降功耗）|

**我们保留了起点初心**：LiteLLM 做成纯网关后，配置下发不是它的重点；我们**网关 + 配置下发一体**，下发能力（10 adapter）是保留的核心，不是附属。

---

## 七、抄什么 / 不抄什么

### ✅ 抄（纯 Rust 路由核心）
- `router-core/`：deployment 注册、cooldown、权重、延迟感知、健康检查
- `gateway/`：axum 网关骨架

### ❌ 不抄
- Python 部分（历史包袱）
- 企业级复杂度（多租户/cost/Redis）——我们个人自用

---

## 八、一句话总结

> **两者都从"配置管理"起家，被痛点逼着长出网关，现在都在往"Rust 高性能路由"走。**
> LiteLLM 背着 Python 包袱渐进迁移（双入口并存）；我们无包袱，从配置下发器直接长成全 Rust 一体化网关（网关 + 下发），一步到它正在迁往的终点。

---

## 附：源码位置（浅克隆 `/mnt/shared/ref-project/api/litellm/`）

- Python proxy（老入口）：`litellm/proxy/proxy_server.py`
- Rust 网关（新入口）：`litellm/rust/crates/gateway/src/main.rs`（axum serve）
- 路由核心：`litellm/rust/crates/router-core/src/`
- 架构决策：`litellm/rust/docs/adrs/adr_001_only_type_inference_inside_rust.md`
- 路由设计细节：见 `docs/LiteLLM路由核心研究.md`
- 通道设计讨论：见 `docs/通道设计讨论-供应商多种接入方式.md`
