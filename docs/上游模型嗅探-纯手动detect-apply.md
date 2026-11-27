# 上游模型嗅探与变更检测（纯手动，学 new-api）

> 用户铁律：**纯手动**——点按钮才打上游，**绝不自动定时嗅探**（"你一直打上游这是绝对不行的"）。
> 参考：new-api `controller/channel_upstream_update.go`（Detect/ApplyChannelUpstreamModelUpdates）。

## 为什么纯手动
- 上游 `/models` 频繁被轮询 = 浪费上游配额 + 可能触发上游限流/封禁。
- new-api 虽内置 30min 定时任务，但 myapi 明确不做定时——只有用户点击「检测变更」才打一次上游。

## 机制（学 new-api，删掉定时/auto-sync）

### 1. 嗅探 `gateway::fetch_upstream_models`（已有）
- `GET {base}/models`（OpenAI 兼容）；解析 `{data:[{id}]}` / `{models:[{id|name}]}` / 裸数组。
- 带 `Authorization: Bearer key` + 自定义 headers。
- 协议扩展（Ollama/Gemini 专门 API）暂未做，后续按 `ChannelModel.protocol` 拼 URL。

### 2. 差异检测 `gateway::diff_upstream_models`（新增）
学 new-api `collectPendingUpstreamModelChangesFromModels`：

```
add    = upstream − local − ignored（ignored 支持 `regex:` 前缀正则）
remove = local − upstream（本地有、上游已消失）
ignored = 被忽略规则命中的上游新模型（提示但不加入）
```
- 忽略规则：`"regex:.*-preview"` 走正则；无前缀走**精确匹配**。
- 无 redirect（myapi 无模型重定向，new-api 的 redirect 豁免不需要）。

### 3. 应用 `gateway::apply_upstream_models`（新增）
- 只写用户**勾选**的 `add`（完整 ChannelModel JSON）/`remove`（模型名）。
- 写回 `gateway.yaml`（保留其他段），热加载即时生效。

### 4. 端点（纯手动）
| 方法 | 路径 | 语义 |
|------|------|------|
| POST | `/api/providers/upstream-models` | detect：读 gateway.yaml 拿 base_url/key/local → 嗅探 → 返回 `{add, remove, ignored, upstream_count, local_count}`（**只检测不写库**）|
| POST | `/api/providers/apply-upstream-models` | apply：`{provider, channel_id, add:[], remove:[]}` → 写回 gateway.yaml → 返回 `{providers}` |

## 前端（Providers.tsx）
- 通道模型映射区新增「🔍 检测变更」按钮（detect）。
- diff 对话框：展示 add（上游新增）/remove（上游已消失）/ignored（被忽略）。
  - **默认 add 全选、remove 不选**（安全：删除需用户确认，学 new-api auto-sync 只加不删）。
  - 忽略规则输入框（逗号分隔，支持 `regex:` 前缀）。
- 「应用」→ 只写勾选的 → 用返回 providers 更新本地状态。
- 原「↓ 拉取模型」（快速加新模型）保留不动。

## 与 new-api 的差异（刻意删掉的）
| new-api | myapi | 原因 |
|---------|-------|------|
| 定时任务 30min（系统任务框架） | **无** | 用户铁律：纯手动 |
| auto-sync 自动写库 | **无**（detect 只检测，apply 手动） | 用户铁律：改库需人工确认 |
| per-channel `check_enabled`/`auto_sync_enabled` | 无（detect 按钮即触发） | 简化，手动入口天然克制 |
| 最小检查间隔 300s | 无 | 纯手动无打上游风险 |
| redirect 别名豁免 | 无 | myapi 无模型重定向 |

## 测试
- `diff_upstream_add_remove_and_regex_ignored`：add/remove/ignored（regex: 命中进 ignored 不进 add）。
- `diff_upstream_no_change_and_empty_rules`：完全一致无差异。
- `diff_upstream_exact_rule_not_regex`：无 `regex:` 前缀走精确匹配。

## 待办（后续可选）
- 按 `ChannelModel.protocol` 拼嗅探 URL（anthropic `/v1/models`、gemini `/v1beta/models`、ollama `/api/tags`）。
- 持久化忽略规则到 Provider（当前对话框内输入，纯会话）。
- 前端 e2e 补 detect/apply 流程（当前 226 Rust 测试 + 前端 typecheck 过，e2e 未含此新流程）。
