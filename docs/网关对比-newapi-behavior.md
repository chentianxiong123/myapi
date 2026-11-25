# myapi 与 new-api 行为/性能对比（持续记录）

> 用成熟项目做独立参照物，避免"自己跟自己比"。本机不跑 new-api（避免污染），在 win11 `C:\newapi-baseline` 跑。
> 测试全程假数据：本地 mock 上游、假 key、假模型，不碰真上游、不花钱。
>
> **教训**：不要没查清就下结论（如误判 new-api 不记失败）。new-api 是成熟项目，行为都有开关/文档，先查源码/issue 再断言。

## 测试环境
- **new-api**：`v1.0.0-rc.51`（Windows exe 138MB），SQLite，端口 3000，计划任务 SYSTEM 启动
- **myapi**：本机 Rust debug 版，端口 8317
- **mock 上游**：win11 `127.0.0.1:9999`，Python http.server 返回假 OpenAI 响应带 usage
- 同流量：3 成功 + 2 失败

## 性能对比

| 维度 | new-api | myapi | 结论 |
|---|---|---|---|
| 二进制体积 | 138MB | ~10MB | myapi 轻 |
| 空载内存 RSS | **52MB** | **8.5MB** | myapi 轻 6 倍 |
| 语言 | Go | Rust | 静态编译，无运行时 |

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

## 待办
- [x] myapi 补 Gemini usage 统计（usageMetadata 字段）— 已完成 119 测试全绿
- [ ] 考虑 Gemini completion=0 兜底（本地估算）
- [ ] 补更多对比维度：重试/熔断行为、SSE 处理、并发性能
