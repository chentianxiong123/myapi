# Python 假测试框架（全流程，一切都要假）

> 用户要求：假上游、假客户端**一切都要假**，**全流程**，各种混协议/内容/特性，用 Python 造多点假测试。
> 定位：比 Rust 单测（wiremock）更接近真实链路——真起 myapi 二进制 + 真 HTTP 打假上游。

## 运行

```bash
# 1. 编译 myapi 二进制（本机实测例外，用户已允）
cd rs && cargo build

# 2. 跑全部场景（10 个）
cd tests/py && python3 run_all.py

# 3. 跑单个场景
python3 run_all.py protocol_matrix
```

## 目录

| 文件 | 作用 |
|------|------|
| `fake_upstream.py` | 可配置假上游：多协议（chat/responses/messages/gemini/ollama）+ `/v1/models` 嗅探 + 内容（文本/工具/思考）+ 错误注入（500/`fail_first_n`/关键词/context 按模型）+ 延迟 |
| `fake_client.py` | 多协议请求生成（chat/responses/messages）+ 内容（图/工具/思考/多轮/长上下文） |
| `framework.py` | `Harness`：起假上游 + 起网关（`MYAPI_ROOT` 隔离）+ 请求/断言 + SQLite 日志读取 |
| `run_all.py` | 10 个全流程场景 |

## 场景（10 个，全通过）

| 场景 | 覆盖 |
|------|------|
| `protocol_matrix` | 混协议：客户端 messages/responses/chat → 上游收对应协议模型 |
| `tier_routing` | 档位 auto/pro/flash + 空候选 404 |
| `capabilities` | 能力硬约束：vision/thinking 无能力→409、有能力→200、disabled→200 |
| `content_multi_turn` | 多轮工具历史 + 长上下文 + 日志对齐 |
| `failover_circuit` | 上游 500→failover 下一通道、全废→502 |
| `sticky_multi_device` | 多设备 key 鉴权（手机/电脑通过、错 key→401） |
| `upstream_sync` | 纯手动嗅探 detect/apply：add 检测 + `regex:` 忽略 + 写回 gateway.yaml |
| `streaming_matrix` | 流式 SSE 混协议（chat/responses/messages 内容 + `[DONE]`） |
| `log_real_model` | 日志记真实模型名（gpt-4o）+ 非流式响应回显档位名（auto） |
| `context_feedback` | context 错误反馈：超长→跳过装不下通道→换大窗口通道 |

## 隔离（不污染真实配置）

- `MYAPI_ROOT=<tempdir>` 环境变量（`main.rs`/`api.rs` 的 `root()` 可覆盖）——网关读临时目录 `config/gateway.yaml`，**不碰真实 `rs/config/`**。
- 假上游配置写入临时目录 `upstream_cfg.json`。
- 每次 `Harness` 用独立端口（upstream 19999、gateway 17840+偏移）。

## 已知坑（写场景时踩过）

1. **base_url 约定**：openai/responses 通道 base 含 `/v1`（转发 `{base}/chat/completions` → `/v1/chat/completions`）；anthropic 通道 base **不含** `/v1`（转发 `{base}/v1/messages`）。混协议用**分开 provider**。
2. **`class` 是 Python 关键字**：模型档位用 `**{"class": "pro"}`。
3. **流式内容编码**：SSE 响应无 charset，`requests.text` 按 latin-1 解码乱码——用 `r.content`（UTF-8 bytes）断言；chat 直传可能 `\uXXXX` 转义。
4. **auth 配置在顶层**（`auth.api_keys`），不是 `gateway` 段。
5. **`fail_first_n` 与 `error` 互斥**：`error` 配置会让所有请求失败，测 failover 只用 `fail_first_n`。
6. **context 按模型限制**：`context.model_limits.{model}` 模拟"某通道装不下、某通道装得下"。

## 与 Rust 测试的分工

- **Rust 单测（win11，228）**：协议转换/调度/熔断等单元级 + wiremock 端点。
- **Python 假测试（本机，10 场景）**：全流程真链路（真 myapi + 真 HTTP + 假上游）。
