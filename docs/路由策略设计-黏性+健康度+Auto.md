# 路由策略设计：同模型自动切换 + 黏性省token + 健康度 + Auto模式

> 目标（用户拍板）：同模型内部**自动路由切换**（选最好的）、**黏性路由省token**、**健康度判断**、**可配置路由策略**、未来**全自动 Auto 模式**（调集一切可用模型，自动路由）。
> 算法自己把握。本文研究三家实现（LiteLLM / 9router / cc-switch），提炼可抄设计。

---

## 一、三家路由策略对比

### ① LiteLLM（最完整，我们主要参考）

**路由策略枚举**（`routing_strategy`）：
| 策略 | 含义 |
|---|---|
| `simple-shuffle` | 简单随机（默认）|
| `least-busy` | 最少并发/最闲 |
| `latency-based-routing` | **延迟最低优先**（TTFT 滑动窗口）|
| `usage-based-routing` | 按用量（rpm/tpm）分配 |
| `cost-based-routing` | 按成本（我们不要）|

**黏性路由（deployment affinity = session stickiness，省 token 关键）**：
源码 `router_utils/pre_call_checks/deployment_affinity_check.py`，三种粒度（独立可开关）：
1. **Responses API 连续性**：有 `previous_response_id` → 路由到生成原始响应的那个 deployment（**最高优先级**）
2. **API-key 亲和**：API key hash → deployment id，TTL 内复用同一 deployment（alias 安全）
3. **会话亲和**：session_id → deployment，TTL 内复用

**设计精髓**（专为 implicit prompt caching，无显式 cache_control）：
> 路由到一致的 deployment，能让上游的 prompt cache 命中 → **省 token / 省钱 / 降延迟**。
> 即使没有显式 cache_control，黏性路由也有收益。

**被动熔断**（`cooldown_handlers.py`）：
- 滑动窗口（按分钟）统计每 deployment 成功/失败
- 失败率 > 50% 且流量足够 → 冷却 5s
- 429 → 直接冷却
- 单 deployment 组保护（不轻易全熔）

### ② 9router（combo 链式 fallback + Auto 模式）

**combo（模型组合）**：`owned_by === "combo"` 的模型，链式串联多个 provider，自动 fallback：
- **Hard capabilities**（硬能力）：vision/pdf/audioInput/videoInput，缺失会**丢请求数据**（如图片被剥离）→ 必须优先匹配
- **Soft capabilities**（软能力）：如 search，缺失只降级功能
- provider 顺序：OAuth 优先 → Free → API Key

**对应我们的 Auto 模式**：combo = 调集多个 provider 链式兜底，一个挂了自动切下一个。这就是"全自动 auto 路由"的雏形。

**工具历史扁平化**（panel 模型不能 loop 工具）：把 tool 调用压成纯文本，保留上下文但去掉工具循环。

### ③ cc-switch（端点测速，最简单）

- **EndpointSpeedTest**：在 provider 卡片测端点延迟，按 `latency` 升序排序（快的优先）
- 无自动路由/failover，只是**手动测速参考**

---

## 二、我们需要的路由能力（逐条设计）

### 需求 1：同模型内部自动路由切换（选最好的）
**场景**：deepseek-v4-flash 有多条通道（SenseNova-ipv4 / SenseNova-ipv6 / pie-xian），自动选最好的打。

**设计**：
- 默认策略可配（延迟感知 / 权重 / 随机 / 最闲）
- 每次请求前，按策略从健康通道里选一条
- **延迟感知**：维护每通道 TTFT 滑动窗口，选最低的（带随机采样防雪崩，学 LiteLLM `lowest_latency_buffer`）

### 需求 2：黏性路由省 token（核心）
**场景**：同一个会话/同一个 key 的连续请求，路由到同一通道 → 上游 prompt cache 命中 → 省 token + 降延迟。

**设计**（学 LiteLLM 三种粒度）：
1. **会话亲和**：session_id → 通道，TTL 内复用（最高优先）
2. **API-key 亲和**：上游 key hash → 通道，TTL 内复用
3. **连续性**：有 previous_response_id → 回到原通道

**TTL 可配**（如 5 分钟 / 30 分钟）。**收益**：即使无显式 cache_control，黏性也有收益（上游隐式缓存）。

### 需求 3：健康度判断
**场景**：路由要避开不健康的通道。

**设计**（学 LiteLLM cooldown，被动统计）：
- 每通道维护滑动窗口（成功/失败/429/TTFT）
- 失败率 > 阈值（默认 50%）→ 熔断（cooldown）
- 429 → 直接冷却
- **staleness**：健康状态有时间戳，过期不算（避免永久拉黑）
- **全挂兜底**：所有通道都不健康 → KeepAll 降级继续用（别一熔就全瘫）

### 需求 4：可配置路由策略
**场景**：用户能配策略和参数。

**设计**（前端配置区，后端读取）：
```yaml
routing:
  strategy: latency-based   # latency-based / weight / simple-shuffle / least-busy
  sticky:
    enabled: true
    ttl_seconds: 300        # 会话亲和 TTL
    granularity: session    # session / api-key / continuity
  cooldown:
    failure_threshold_percent: 0.5   # 失败率阈值
    cooldown_seconds: 5              # 冷却时长
    min_requests: 5                  # 失败率有意义的最小请求数
  when_all_unhealthy: keep-all       # keep-all / fail
  latency_window_seconds: 60         # TTFT 滑动窗口
```

### 需求 5：全自动 Auto 模式（未来）
**场景**：一个"auto"上游模型，调集一切可用模型，自动路由。

**设计**（学 9router combo）：
- 一个 alias 叫 `auto`，背后是**所有健康通道**的链式池
- 请求来了 → 按 capability 匹配（vision/搜索/嵌入）→ 按 strategy 选最优 → 失败 fallback 下一个
- **capability 匹配**：Hard（vision/pdf）必须满足，Soft（搜索）可降级
- 类似 9router 的 `owned_by: combo`，但更智能（按实时健康+延迟动态排序，非固定链）

---

## 三、路由决策流程（综合）

```
请求进来（model=deepseek-v4-flash, session_id=xxx）
  ↓
1. 黏性检查：session_id 有 pin 且未过期且通道健康？→ 用 pinned 通道（省 token）
  ↓ 否
2. 健康过滤：从该模型的所有通道里，过滤掉熔断中的
  ↓
3. 若全熔断 → when_all_unhealthy: keep-all → 保留全部（降级）
  ↓
4. 按策略选一条：
   - latency-based → 选 TTFT 最低（随机采样 top-N 防雪崩）
   - weight → 按权重随机
   - least-busy → 选并发最低
  ↓
5. 记录 pin（session_id → 通道，TTL）
  ↓
6. 转发 → 记录结果（成功/失败/TTFT）→ 更新滑动窗口 → 可能触发熔断
```

---

## 四、前端要补的配置区（路由策略）

放哪：Config 页新增「路由策略」区，或独立「路由」页。

```
路由策略
├── 策略：[延迟感知 ▼]（延迟感知/权重/随机/最闲）
├── 黏性路由
│   ├── 启用 [✓]
│   ├── 粒度：[会话 ▼]（会话/API-key/连续性）
│   └── TTL：[300] 秒
├── 熔断
│   ├── 失败率阈值：[50]%
│   ├── 冷却时长：[5] 秒
│   └── 最小请求数：[5]
├── 全挂兜底：[降级继续(keep-all) ▼]
├── 延迟窗口：[60] 秒
└── Auto 模式（未来）
    ├── 启用 [ ]
    └── capability 匹配：vision/pdf 优先
```

---

## 五、优先级（实现顺序）

1. **黏性路由**（省 token，用户强调）→ 后端核心
2. **健康度 + 熔断**（被动统计）→ 后端核心
3. **可配置策略**（前端配置区 + 后端读取）
4. **延迟感知**（TTFT 滑动窗口）
5. **Auto 模式**（未来，capability 匹配 + 动态排序）

---

## 六、给后端 Rust 的落地（照 LiteLLM router-core）

| LiteLLM | 我们的 |
|---|---|
| `deployment_affinity_check.py` | 黏性路由（session pin + TTL）|
| `cooldown_handlers.py` | 熔断（滑动窗口 + 失败率阈值）|
| `health_routing.rs` | staleness + when_all_unhealthy |
| `strategy.rs` / `weights.rs` | 路由策略 + 权重 |
| `prompt_caching_deployment_check.py` | prompt cache 命中优化 |

---

## 七、待拍板

1. 路由策略配置放 **Config 页**还是**独立「路由」页**？
2. 黏性粒度默认 **session** 还是 **api-key**？（我倾向 session，对话场景）
3. Auto 模式是**未来做**还是**现在就设计进去**？（我倾向现在留接口，未来实现）

> 详细实现见 `docs/LiteLLM路由核心研究.md`；通道模型见 `docs/通道设计讨论-供应商多种接入方式.md`
