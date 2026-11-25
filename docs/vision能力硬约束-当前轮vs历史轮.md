# Vision 能力硬约束：当前轮 vs 历史轮

> 问题：有的模型支持识图，有的不支持。请求原样发图片上去会 400。网关怎么兜底？
> 结论：**区分「当前轮」和「历史轮」——当前轮硬失败（不静默），历史轮软降级（剥离占位）。**
> 本文对照 9router / omp / LiteLLM 源码，给出 myapi 的实现方案。

## 一、问题本质：静默降级是失守

最直觉的做法是「模型不吃图就把图剥掉发纯文本」（strip）。但这是**失守**：

```
客户端发图 → 网关静默 strip → 发纯文本给非 vision 模型 → 返回"没看图"的回答
客户端以为模型看了图，其实没有 → 用户/agent 拿到错误结论
```

**极端情况（都会失守）**：
1. 客户端**交替使用**能识图和不能识图的模型——历史里混着图，当前轮不带图，但模型选了非 vision
2. 客户端**想要识图**，但塌缩/兜底一直返回不能识图的模型——静默 strip 后客户端无从得知

**失守根源：把「当前轮的能力需求」当软需求静默降级。**

## 二、核心区分：当前轮 vs 历史轮

```
带图请求
  ├─ 图在「当前 user turn」→ 硬需求（客户端此刻明确要识图）
  │     ├─ 有 vision 通道 → 【A 硬过滤】发完整请求（带图）给能看的模型
  │     └─ 无 vision 通道 → 【显式失败】HTTP 409 "no vision-capable model available"
  │                        ↑ 不静默！客户端知道图发不上去，可改模型/重试
  │
  └─ 图在「历史轮」→ 软需求（旧上下文，不是当前要看的）
        └─ 【B strip + C 占位符】删图片块换 `[image omitted]`，发纯文本
           ↑ 不报错，客户端无感；模型知道"这里本该有图"
```

| 场景 | 处理 | 客户端感知 |
|---|---|---|
| 当前轮带图 + 有 vision 模型 | A 硬过滤，发给 vision 模型 | 正常识图 ✓ |
| 当前轮带图 + 无 vision 模型 | **409 显式失败** | 知道图发不上去，可重试 ✓ |
| 历史轮带图 + 当前轮不带图 | strip + 占位符 | 无感，回答正常 ✓ |
| 当前轮 + 历史轮都带图 | 当前轮按硬需求，历史轮剥离 | 识图 + 历史降级 ✓ |

**交替使用、兜底模型不能改——两个极端情况都被「当前轮显式失败」堵住**：客户端永远不会拿到一个"自以为看了图"的回答。

## 三、三个机制各归位（A/B/C）

| 机制 | 作用 | 何时用 |
|---|---|---|
| **A 硬过滤** | 请求带图 → 只选有 vision 的通道 | 当前轮带图，有 vision 可用 |
| **显式失败 409** | 无 vision 可用 → 报错，不静默 | 当前轮带图，无 vision 可用 |
| **B strip** | 删 `image_url`/`image` 内容块 | 历史轮带图 |
| **C 占位符** | 换成 `[image omitted: model does not support vision]` | 历史轮带图（B+C 合并） |

**A 优先（能看就发图），409 兜底（当前轮不能看就报错），B+C 降级（历史轮剥离占位）**。

## 四、对照参考项目

### 9router（最完整，本方案的主要参照）
- `open-sse/services/combo.js`：
  ```js
  // Hard capabilities = input modalities; missing one drops request data (e.g. image
  // stripped). Must be prioritized. Soft (e.g. search) only degrades a feature.
  const HARD_CAPS = new Set(["vision","pdf","audioInput","videoInput"]);
  ```
- **`detectRequiredCapabilities`：只扫当前 user turn**——这就是"当前轮 vs 历史轮"的区分：
  ```js
  // Modalities (vision/pdf) are scanned only on the current user turn;
  // History media (older turns) must not pin the combo to a vision model —
  // those get stripped + placeholdered downstream instead.
  ```
- `open-sse/translator/index.js::stripContentTypes`：按 `PROVIDER_MODELS` 的 `strip[]` 显式删图片块。
- `reorderByCapabilities`：Tier0 全满足 / Tier1 硬能力满足 / Tier2 不满足（stable sort，不丢模型，保 fallback）。

### omp / pi-coding-agent（`@oh-my-pi\pi-coding-agent\dist\cli.js`）
- 能力检测：`input.includes("image")`
- 占位符：`"[image omitted: model does not support vision]"`
- `media_sanitizer.replace_images_for_text_only_model`：把图换成占位符
- `compat.stripImageInput`：强制剥离
- 上游拒绝图片时重试（`[Media] Upstream rejected image input; retrying ...`）

### cc-switch
**无此能力**——cc-switch 是 client 侧配置管理器（改写 `~/.codex/config.toml` 等），**不转发请求**，不参与请求处理。其 `capabilities` 字段是 MCP tools 相关，与 vision 无关。

### cpa / myapi（现状）
已有机制 A 的骨架（`rs/src/gateway.rs`）：
- `ChannelModel.capabilities`：硬能力标注（vision/pdf/audio，学 9router HARD_CAPS）
- `detect_required_caps`：扫 `messages[].content[]` 的 `image_url`/`file`
- `auto_candidates`：硬能力匹配，缺 vision 不接带图请求

**缺**：当前轮/历史轮区分（现在扫全部 messages）、无 vision 时的显式 409、历史轮 strip 占位。

### LiteLLM
- `/model/info` 的 `supports_vision` + `litellm.utils.supports_vision()`
- 但**不做能力路由**（不自动避开非 vision 模型）——靠客户端自己保证

## 五、实现方案（myapi）

### 5.1 `detect_required_caps` 改成只扫当前轮
```rust
/// 只扫当前 user turn（最后一段连续的 user message，到最后一个 assistant 为止）。
/// 历史轮的图/pdf 属于软需求，走 strip，不 pin 住能力选择。
fn trailing_user_turn(msgs: &[Value]) -> &[Value] { ... }
```
（9router `trailingUserItems` 同款：从末尾回退到最后一个 assistant/model 角色）

### 5.2 当前轮能力不满足 → 409
```
HTTP 409
{
  "error": {
    "type": "capability_unavailable",
    "message": "No vision-capable model available for image request",
    "required": ["vision"]
  }
}
```
**不静默**——客户端（codex/claude）收到 409 就知道图发不上去，可换模型/重试。

### 5.3 历史轮 strip + 占位符
```rust
/// 转发前处理：历史轮的 image_url/file 块 → 占位符文本
fn strip_history_media(messages: &mut Vec<Value>, supported: &HashSet<String>) {
    // 非当前轮的 image 块：{"type":"text","text":"[image omitted: model does not support vision]"}
}
```

### 5.4 配置项
```yaml
providers:
  - channels:
      - models:
          - name: sensenova-6.8-flash-lite
            capabilities: [vision, pdf]   # 硬能力标注
```
strip 可显式关闭（某些网关要求必须带图）：
```yaml
      - strip: [image]   # 显式剥离图片（等价 capabilities 缺 vision）
```

## 六、一句话
**当前轮是硬需求（不满足就 409，绝不静默），历史轮是软需求（strip + 占位符降级）。**
静默降级当前轮 = 客户端失守；显式失败当前轮 = 客户端永远知道。
