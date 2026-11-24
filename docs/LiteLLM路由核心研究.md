# LiteLLM 路由核心研究（myapi 主要参考）

> 结论先行：**LiteLLM 是最合适 myapi 参考的项目**（用户拍板）。更关键的是——
> **它的路由核心就是 Rust 写的**（`litellm/rust/crates/router-core/`），跟 myapi 后端用 Rust 完全对口。
> 本文提炼最该抄的设计，附源码位置（浅克隆在 `/mnt/shared/ref-project/api/litellm/`）。

---

## 一、为什么 LiteLLM 最合适

| 维度 | LiteLLM | 我们要不要 |
|---|---|---|
| **deployment 全绑死**（key×base×映射） | ✅ `Deployment { model_name, litellm_params, model_info }` | ✅ 正是三个刁钻场景要的 |
| **被动冷却**（非主动探测） | ✅ cooldown 由真实请求失败触发 | ✅ 用户明确要被动 |
| **熔断按 deployment 粒度** | ✅ 单个 deployment 冷却，不影响其他 | ✅ 要的 |
| **多种路由策略** | ✅ simple-shuffle / least-busy / latency-based / usage-based | ✅ 参考 |
| **Rust 路由核心** | ✅ `rust/crates/router-core/` | ✅ 跟我们后端 Rust 对口 |

对比 new-api（渠道 1:1 + 主动定时测）——LiteLLM 在"细粒度 + 被动"两点上都更贴合我们。

---

## 二、Deployment：三件套绑死（对应我们的"通道"）

**源码**：`litellm/types/router.py:921`

```python
class Deployment(LiteLLMBaseModel):
    model_name: str              # 对外别名（如 deepseek-v4-flash）
    litellm_params: LiteLLM_Params  # ← 三件套：model + api_base + api_key 绑死
    model_info: ModelInfo        # 元信息（id、权重、优先级等）
    blocked: bool | None         # 管理员手动暂停
```

**LiteLLM_Params**（`litellm/types/router.py:778`）：
```python
class LiteLLM_Params:
    model: str          # 上游真实模型名
    api_base: str       # 上游地址（网络路径）
    api_key: str        # 密钥
    # ... provider 特定字段
```

**要点**：`model_name`（对外别名）→ 多个 Deployment（各带 key/base/model 映射），**三件套在 Deployment 内绑死**。
这正是我们"通道"要的：一个供应商 → 多条通道（每条 = key×路径×映射）。

---

## 三、冷却机制：被动熔断（最该抄）

### 触发条件（`router_utils/cooldown_handlers.py:600-630`）
冷却由**真实请求失败**触发（非主动探测），判定逻辑：

```python
# 三种触发（任一满足即进 cooldown）：
# ① 429 限流（非单 deployment 组）→ 直接冷却
if exception_status == 429 and not is_single_deployment_model_group:
    return True
# ② 失败率 100% 且流量足够（≥ SINGLE_DEPLOYMENT_TRAFFIC_FAILURE_THRESHOLD=1000）
elif percent_fails == 1.0 and total_requests >= SINGLE_DEPLOYMENT_TRAFFIC_FAILURE_THRESHOLD:
    return True
# ③ 失败率 > 50%（DEFAULT_FAILURE_THRESHOLD_PERCENT=0.5）
#    且流量 ≥ DEFAULT_FAILURE_THRESHOLD_MINIMUM_REQUESTS
elif percent_fails > 0.5 and total_requests >= MINIMUM_REQUESTS:
    return True
```

### 关键参数（`litellm/constants.py`）
| 参数 | 默认值 | 含义 |
|---|---|---|
| `DEFAULT_FAILURE_THRESHOLD_PERCENT` | **0.5**（50%）| 失败率超过就冷却 |
| `DEFAULT_COOLDOWN_TIME_SECONDS` | **5** 秒 | 冷却时长 |
| `SINGLE_DEPLOYMENT_TRAFFIC_FAILURE_THRESHOLD` | **1000** | 单 deployment 判全挂的流量门槛 |
| `DEFAULT_FAILURE_THRESHOLD_MINIMUM_REQUESTS` | （另一常量）| 失败率有意义的最小请求数 |

**设计精髓**：
- **滑动窗口统计**：按分钟统计每 deployment 的成功/失败数（`get_deployment_failures_for_current_minute`）
- **冷却时长**：`configured_cooldown(deployment)` 优先，否则用 `fails_window()`（滑动窗口时长）
- **单 deployment 保护**：`is_single_deployment_model_group` 时不轻易冷却（避免只有一个可用时全熔）

---

## 四、健康路由：staleness + 全挂策略（Rust）

**源码**：`litellm/rust/crates/router-core/src/health_routing.rs`

```rust
// 不健康 deployment 判定：标记为不健康 且 结果未过期（< staleness）
pub fn unhealthy_ids(states, now, staleness) {
    states.filter(|(_, state)| !state.is_healthy && now - state.timestamp < staleness)
}

// WhenAllUnhealthy 枚举：当所有 deployment 都不健康时怎么办
pub enum WhenAllUnhealthy {
    KeepAll,   // 全保留（降级：都 unhealthy 也照常路由）
    Nothing,   // 全过滤（返回空，触发错误）
}
```

**要点**：
- **staleness（陈旧阈值）**：健康状态有时间戳，超过 staleness 的旧状态不算数（避免用陈旧数据永久拉黑）
- **全挂兜底策略**：所有 deployment 都不健康时，可选 KeepAll（降级继续用）或 Nothing（直接失败）——我们该学 KeepAll

---

## 五、路由策略（Rust `strategy.rs`）

**策略枚举**（`litellm/types/router.py:1321`）：
| 策略 | 含义 | 我们要否 |
|---|---|---|
| `simple-shuffle` | 简单随机（默认）| ✅ 基础 |
| `least-busy` | 最少并发/最闲 | 可选 |
| `latency-based-routing` | **延迟最低优先** | ✅ **核心（延迟感知）** |
| `usage-based-routing` | 按用量（rpm/tpm）分配 | 参考 |
| `cost-based-routing` | 按成本（我们不要成本）| ❌ |

**StrategyScope**（`rust/.../strategy.rs:23`）：策略可 **Override**（请求级覆盖）或 **Default**（默认）或 **Group**（按模型组）。

---

## 六、权重路由（Rust `weights.rs`）

**结构**：`{ 模型组: { deployment_id: 权重 } }`
```json
{ "deepseek-v4-flash": { "deployment-a": 80, "deployment-b": 20 } }
```
- 权重 ≥ 0（int 或 float），每非空组至少一个正权重
- `validate_router_weights` 校验合法性

**对应我们**：模型（alias）按权重分流到多个通道（通道①80% + 通道②20%）。

---

## 七、myapi 落地映射（LiteLLM → 通道设计）

| LiteLLM 概念 | myapi 对应 | 备注 |
|---|---|---|
| `model_name`（对外别名） | 模型 alias | deepseek-v4-flash |
| `Deployment`（三件套） | **通道**（key×路径×映射）| 供应商的子层 |
| `api_key` | 通道.key | 一通道一 key |
| `api_base` | 供应商.base_url + 通道.网络路径（IPv4/IPv6）| 路径在通道级 |
| `weight` | 通道.权重 | 模型分流 |
| cooldown（被动） | 通道.熔断 | 被动统计 |
| latency-based-routing | 延迟感知路由 | TTFT 低的通道优先 |
| WhenAllUnhealthy::KeepAll | 全挂降级 | 全熔时兜底 |

**映射到三个刁钻场景**：
1. deepseek 大写/小写映射 → 同 `model_name`，多个 Deployment 各映射不同上游名
2. websearch IPv4/IPv6 → 同供应商多个 Deployment，`api_base`/路径不同
3. VIP/普通 key → 同供应商多个 Deployment，`api_key` 不同、覆盖模型不同

---

## 八、给后端 Rust 的直接借鉴

LiteLLM `rust/crates/router-core/` 的模块划分可直接参考：
- `deployment.rs` / `registry.rs` → 我们的通道注册
- `health.rs`（冷却 + 用量窗口）/ `health_routing.rs`（staleness + 全挂策略）→ 被动熔断
- `strategy.rs` / `weights.rs` → 路由策略 + 权重
- `select.rs` / `prerouting.rs` → 选 deployment 主逻辑
- `fallback.rs` → 降级

**性能优势**：LiteLLM 把路由核心从 Python 迁到 Rust，就是为了低延迟 + 低内存。
这跟我们"替代 new-api+CPA 双进程、降功耗"的目标完全一致——**Rust 单进程 + Rust 路由核心**。

---

## 九、下一步

1. 用户拍板命名（通道/渠道）+ 确认 `Provider.keys` 废掉 + 模型页纯只读
2. 改 `types.ts`：Provider → 多通道（通道 = key×路径×映射×权重）
3. 重构供应商页（配置中心：建供应商 + 建通道）
4. 重构模型页（健康度台：被动统计 + 熔断 + 路由分流 + 通道来源）
5. 后端 Rust：参考 `router-core/` 实现 deployment 注册 + 冷却 + 路由选择

> 详细讨论过程见 `docs/通道设计讨论-供应商多种接入方式.md`
