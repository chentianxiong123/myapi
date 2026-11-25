# myapi 与 new-api 行为/性能对比（持续记录）

> 用成熟项目做独立参照物，避免"自己跟自己比"。本机不跑 new-api（避免污染），在 win11 `C:\newapi-baseline` 跑。
> 测试全程假数据：本地 mock 上游、假 key、假模型，不碰真上游、不花钱。
>
> **教训**：不要没查清就下结论（如误判 new-api 不记失败）。new-api 是成熟项目，行为都有开关/文档，先查源码/issue 再断言。

## 测试环境
- **new-api**：`v1.0.0-rc.51`（Windows exe 138MB），SQLite，端口 3000，计划任务 SYSTEM 启动
- **LiteLLM proxy**：`v1.111.0`（pip venv 装），端口 4000，计划任务 SYSTEM 启动；无 DB 模式
  （spend/logs 需 PostgreSQL，本地 mock 测试不装，故日志明细为已知限制）
- **myapi**：本机 Rust debug 版，端口 8317
- **mock 上游**：win11 `127.0.0.1:9999`，Python http.server 返回假 OpenAI 响应带 usage
- 同流量：3 成功 + 2 失败
- 全程假数据：本地 mock、假 key、假模型，不碰真上游不花钱；cc-switch 本机只读参考绝不碰

## 性能对比（三网关，同 win11 实测）

| 维度 | LiteLLM proxy | new-api | myapi | 结论 |
|---|---|---|---|---|
| 语言/运行时 | Python（uvicorn） | Go | Rust | myapi 无运行时 |
| 二进制/安装 | pip 装（~500MB 依赖） | 138MB exe | ~10MB | myapi 最小 |
| **空载内存 RSS** | **503MB** | **28MB** | **8.5MB** | **myapi 比 LiteLLM 轻 60 倍** |
| 日志存储 | 需 PostgreSQL（SQLite 不支持） | SQLite 内置 | SQLite 内置 | myapi/new-api 自包含 |
| 端口 | 4000 | 3000 | 8317 | — |

**重磅**：LiteLLM（Python）空载就吃 503MB——拖整个 Python 运行时 + fastapi/pydantic/
openai 等依赖。myapi 静态编译 8.5MB，**轻 60 倍**。个人网关/手机场景 LiteLLM 完全不适用。

> 注：new-api 内存从最早测的 52MB 降到 28MB（不同版本/空闲状态波动），量级一致：
> Go 网关几十 MB，Rust 个位数 MB，Python 数百 MB。

## 行为对比

### 日志记录
| 能力 | new-api | myapi | 差异 |
|---|---|---|---|
| 成功请求日志 | ✅ type=2 消费日志（model/prompt/completion/quota） | ✅ status/token/provider/channel | 一致 |
| 上游失败日志 | ✅ type=5 错误日志，**需 `ERROR_LOG_ENABLED=true`（默认 false）** | ✅ 默认就记（status+error） | new-api 要配开关；myapi 默认开 |
| 前置失败（无可用渠道/模型不存在 503） | ❌ **不写日志行**（只有 stdout） | ✅ 记 | **myapi 多记**（可观测定位） |
| 失败计费 | 不扣费 | 无计费概念 | 定位不同 |

**关键细节（源码/issue 证实）**：
- new-api 错误日志开关是环境变量 `ERROR_LOG_ENABLED`（issue #1138）
- new-api "无可用渠道"前置失败默认不写 logs 表（issue #8091 在请求改进）
- new-api 日志类型：1=充值 2=消费 3=管理 4=系统 5=错误

### usage 统计算法（特殊协议差异）⚠️
| 协议 | new-api 算法（源码） | myapi 现状 | 差异 |
|---|---|---|---|
| **OpenAI** | `usage.prompt_tokens/completion_tokens` | ✅ 认 | 一致 |
| **Anthropic** | `usage.input_tokens/output_tokens` + cache 字段 | ✅ 认（parse_usage_json） | 一致 |
| **Gemini** | `buildUsageFromGeminiMetadata`：`usageMetadata.promptTokenCount/candidatesTokenCount` | ✅ 已补（parse_usage_json 认 usageMetadata） | 已对齐 |
| Gemini completion=0 补齐 | `patchGeminiZeroCompletionUsage`：上游报 0 时本地估算 | ❌ 无 | new-api 有兜底 |
| 图像生成 | `imageTokens * generatedImages`（每图 258 token） | ❌ 无 | 特殊场景 |

**结论**：myapi 已补齐 Gemini usage 统计（usageMetadata/promptTokenCount/candidatesTokenCount/cachedContentTokenCount），与 new-api 对齐。wiremock 真测通过。

### cache token 口径差异（Claude 三段式）
| | new-api | myapi |
|---|---|---|
| input_tokens | 含 cache 的总输入（OpenAI 风格合并，`cacheCreation = PromptTokens - input - cached`） | 纯 input（Anthropic 原始） |
| cache_read/creation | 合并进 PromptTokens 计费 | **独立三列保留** |
| total_input 公式 | prompt_tokens 已含 cache | `cache_read + cache_creation + input`（Anthropic 官方口径） |

**Anthropic 官方**：`total_input = cache_read_input_tokens + cache_creation_input_tokens + input_tokens`。
new-api 为计费合并（PromptTokens 要算钱）；myapi 为观测拆开存（能直接算 cache 命中率）。
**两种都对，目的不同**，myapi 忠实保留原始三段，不低估不漏。

### 统计口径
| 维度 | new-api | myapi |
|---|---|---|
| 请求计数 | 只计成功消费（quota/rpm/tpm） | 计所有请求（成功+失败分开） |
| 定位 | 计费网关（关心花多少钱） | 可观测网关（关心发生了什么） |

## 配置/运维
| 维度 | new-api | myapi |
|---|---|---|
| 配置方式 | Web UI + 环境变量 + DB | gateway.yaml（热加载）+ Web |
| 数据库 | SQLite/MySQL/PG/ClickHouse | SQLite |
| Redis | 可选 | 无（个人够用） |
| 多租户 | 有（用户/令牌/分组） | 无（个人） |

### 图像 token 估算（多模态）
各协议图像 token 算法不同（计费用）：
- OpenAI GPT-4o: high `85 + 170×tiles`（512²切块）/ low 85
- Gemini: ≤384px 258/图，否则 `258×ceil(w/768)×ceil(h/768)`
- Claude: `ceil(w×h/750)`

**myapi 取舍**：`estimate_input_tokens` 只算文本（len/4），**不算图像**。
理由：该估算只用于上下文窗口粗筛（max_input_tokens），**不计费不统计**（统计用上游真实 usage）。
粗筛只需量级对，不需图像精度。强行补图像算法=增加复杂度且仍不如上游准。
→ **已知取舍，非缺陷**。多模态请求的图像 token 由上游 usage 回传后如实记录。

### 重试/熔断机制
| 维度 | new-api | myapi | 差异 |
|---|---|---|---|
| 重试开关 | `RetryTimes`（默认 0=不重试） | RetryConfig.max_retries（可配） | myapi 默认可重试 |
| 重试状态码 | `AutomaticRetryStatusCodes`=100-199,300-399,401-407,409-499,500-503,505-523,525-599 | retry_on_status（可配） | new-api 默认范围细 |
| 熔断机制 | **按次数禁用**（`ChannelDisableThreshold`=5 连续失败）+ `AutomaticDisableStatusCodes`(401) | **三态熔断**（closed/open/half-open，可恢复） | **myapi 能自动恢复** |
| 关键词熔断 | `AutomaticDisableKeywords`（匹配"credit balance too low"等文本自动禁渠道） | 无 | new-api 有，myapi 无 |
| 熔断恢复 | 手动启用（disabled 渠道需人工开） | half-open 自动探测恢复 | **myapi 更智能** |

**核心差异**：
- new-api 熔断 = **一次性禁用**（连续 N 次失败→disabled，需人工恢复）
- myapi 熔断 = **三态自动恢复**（open 后 half-open 探测，成功则 closed）
- new-api 独有**关键词熔断**（按错误文本禁渠道，如余额不足），myapi 可考虑借鉴

### 协议/算法差异全景（对照 new-api 源码 + 实测）
| 协议/机制 | new-api 算法 | myapi 状态 |
|---|---|---|
| OpenAI usage | prompt_tokens/completion_tokens | ✅ 一致 |
| Anthropic usage | input_tokens/output_tokens | ✅ 一致 |
| Gemini usage | usageMetadata.promptTokenCount/candidatesTokenCount | ✅ 已补 |
| cache token 口径 | 合并进 PromptTokens（计费） | ✅ 独立三列（观测，更忠实） |
| 图像 token | 各协议精算 | ⚪ 不补（粗筛够用） |
| 重试状态码 | AutomaticRetryStatusCodes 细范围 | ✅ 可配 |
| 熔断 | 按次数一次性禁用 | ✅ 三态（更好，能恢复） |
| 关键词熔断 | AutomaticDisableKeywords（按错误文本禁渠道） | ✅ **已实现**（`retry.disable_keywords`，学 new-api） |

### 关键词熔断（已实现，学 new-api）
myapi 新增 `retry.disable_keywords` 配置（默认词表含 insufficient quota /
credit balance is too low / organization disabled / 余额不足 等）。

**机制**：失败响应体匹配关键词 → 一次命中即强制熔断该通道（`record_outcome`
force_open，绕过失败次数阈值）+ 返 Retryable 触发 failover 切下一个通道。

**vs 错误码熔断**：错误码（500）分不清"临时过载"还是"欠费废了"；关键词直接
看错误文本，欠费/封号一次识别立即切，不浪费时间重试。

**实现要点（踩坑）**：关键词检查必须放在 **5xx 早返回块之前**——原来 5xx 走
`status.is_server_error()` 早返回，永远到不了后面的非流式分支关键词检查（debug
发现 ch-main total_requests=1，分支没执行）。提到早返回前先读 body 检查。

**测试 6 个**：hit_disable_keyword 匹配（大小写/中英）、force_open 绕过阈值、
普通失败不熔断、双 record_outcome 置 Open、e2e 一次命中切通道、普通 500 不误判。

### 流式 SSE usage 适配（跨 chunk 累积）
各协议流式 usage 位置不同，myapi 已全覆盖（对照 new-api）：
| 协议 | 流式 usage 位置 | myapi 处理 |
|---|---|---|
| OpenAI | 最后 chunk `usage.prompt_tokens/completion_tokens` | ✅ 单 chunk |
| Anthropic | **分片**：`message_start.usage.input_tokens` + `message_delta.usage.output_tokens` | ✅ **跨 chunk 累积合并**（修了"取最后一个丢 input"的 bug） |
| Gemini | chunk 顶层 `usageMetadata.promptTokenCount/candidatesTokenCount` | ✅ |

### mock 供应商测试体系（纳入框架）
每种协议都有三层测试，全 wiremock 假上游（不碰真上游）：
- **单元对照**（gateway.rs `proto_*`）：parse_usage_json + sse_scan_usage 逐协议验证
- **非流式 e2e**（main.rs `e2e_mock_*`）：打请求核对统计/日志落库一致
- **流式 e2e**（main.rs `stream_mock_*`）：wiremock 返 SSE，核对跨 chunk usage 落库
合计 129 测试全绿。

## 待办
- [x] myapi 补 Gemini usage 统计（usageMetadata 字段）— 已完成 119 测试全绿
- [ ] 考虑 Gemini completion=0 兜底（本地估算）
- [ ] 补更多对比维度：重试/熔断行为、SSE 处理、并发性能
