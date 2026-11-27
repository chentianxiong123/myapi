// Provider（学 myapi 的 openai-compatibility 结构）
export type ModelProtocol = 'openai_chat' | 'openai_responses' | 'anthropic' | 'gemini' | 'ollama'

// Provider（学 myapi 的 openai-compatibility 结构）
export interface Provider {
  name: string
  base_url: string
  disabled: boolean
  headers: Record<string, string>
  // 密钥池（命名密钥，学 LiteLLM 继承+覆盖）：通道引用名字，不直接填 key
  // 约定：第一个 key 名为 '主'，通道不指定 key_name 时自动用它
  keys: Array<{ name: string; 'api-key': string }>
  // 通道（学 LiteLLM Deployment，三件套绑死：key×路径×映射）
  // 供应商的子层：一个供应商 → 多条通道
  channels: Channel[]
  // 渠道级别代理（每个 Provider 可单独走代理；空=直连/全局代理）
  proxy: string
  // 代理池（学 9router：引用 proxyPools 的一个池 + 选择策略；空=不用池）
  proxy_pool_id: string
  proxy_strategy: 'round-robin' | 'random' | 'none'
}

// 通道内模型映射（学 LiteLLM Deployment model 字段 + 9router 能力标注）
export interface ChannelModel {
  name: string
  alias?: string
  protocol?: ModelProtocol
  kind?: ModelKind
  // 硬能力（vision/pdf/audio/thinking，学 9router HARD_CAPS）：请求需要而模型缺 → 过滤/降级
  // thinking = 思考模式（同 vision 硬约束：请求带 thinking/reasoning 且模型无 → 409）
  capabilities?: string[]
  // 模型级上下文窗口（token）：上下文错误反馈时跳过装不下的（配置时填，运行时零成本）
  context_window?: number
  // 档位标注（对标 Gemini pro/flash）：pro=质量型、flash=速度型；空=auto 专属
  class?: 'pro' | 'flash'
}

// 通道级重试覆盖（学 LiteLLM RetryPolicy；空=继承全局 retry）
export interface RetryOverride {
  max_retries?: number
  retry_on_status?: number[]
  fatal_status?: number[]
  backoff_initial_ms?: number
  backoff_max_ms?: number
  jitter?: number
  respect_retry_after?: boolean
}

// 通道（命名待定，暂用通道/Channel）：供应商的一条具体接入方式
// = 一个 key（引用密钥池）× 一条网络路径 × 一组模型名映射 三件套
// 参考：LiteLLM Deployment（api_key + api_base + model 全绑死在一个 deployment）
export interface Channel {
  id: string
  name: string                  // 通道名（如 带key-ipv4 / 无key-ipv6 / VIP）
  path?: string                 // 网络路径（IPv4/IPv6 描述；空=直连/继承供应商代理）
  key_name?: string             // 引用密钥池的名字（空=用第一个「主」密钥）
  // 该通道覆盖的模型映射：上游真实名 → 可选对外别名
  models: ChannelModel[]
  weight: number                // 路由权重（权重高调用多，默认 1）
  // 每分钟请求上限（RPM，空=不限）；超限当不可用跳过（防打爆上游）
  rpm?: number
  // 通道级最大输入 token（上下文窗口过滤：超长请求跳过装不下的通道）
  max_input_tokens?: number
  // 通道级重试覆盖（空=继承全局 retry）
  retry?: RetryOverride
}

// 模型类型（学 9router /v1/models 分类：chat/webSearch/webFetch/embedding）
export type ModelKind = 'chat' | 'webSearch' | 'webFetch' | 'embedding' | 'vision'

export const MODEL_KINDS: Array<{ id: ModelKind; label: string; endpoint: string }> = [
  { id: 'chat', label: '对话（chat）', endpoint: '/v1/chat/completions' },
  { id: 'webSearch', label: 'Web 搜索（search）', endpoint: '/v1/search' },
  { id: 'webFetch', label: '网页抓取（fetch）', endpoint: '/v1/web-fetch' },
  { id: 'embedding', label: '嵌入（embedding）', endpoint: '/v1/embeddings' },
  { id: 'vision', label: '识图（vision）', endpoint: '/v1/chat/completions' },
]

export const MODEL_PROTOCOLS: Array<{ id: ModelProtocol; label: string; path: string }> = [
  { id: 'openai_chat', label: 'OpenAI Chat', path: '/v1/chat/completions' },
  { id: 'openai_responses', label: 'OpenAI Responses', path: '/v1/responses' },
  { id: 'anthropic', label: 'Anthropic Messages', path: '/v1/messages' },
  { id: 'gemini', label: 'Gemini', path: '/v1beta/models' },
  { id: 'ollama', label: 'Ollama', path: '/api/chat' },
]

// 对外暴露的模型（按 alias 分组）
// 运行态聚合视图：一个 alias 由多条通道供（跨供应商），按权重路由
// 模型页只读展示：健康度/熔断/路由分流/通道来源
export interface ExposedModel {
  id?: string
  alias: string
  kind?: ModelKind
  // 路由到的通道引用（provider_name + channel_name 定位到具体通道）
  routes: ModelRoute[]
}

// 统计概览（学 cc-switch UsageSummary）
export interface StatsSummary {
  total_requests: number
  success_count: number
  success_rate: number
  error_count: number
  error_rate: number
  // Token 数据（学 cc-switch）
  total_input_tokens: number
  total_output_tokens: number
  total_cache_creation_tokens: number  // 缓存写入
  total_cache_read_tokens: number      // 缓存读取
  real_total_tokens: number            // input + output + cache_creation + cache_read
  cache_hit_rate: number               // cache_read / (input + cache_creation + cache_read)
  // 成本
  // 延迟
  avg_ttft_ms: number                  // 首字时间
  avg_latency_ms: number               // 总延迟
  avg_speed_tps: number                // 输出速度（tokens/s）
  // Provider
  active_providers: number
  total_providers: number
}

export interface ProviderStat {
  name: string
  base_url: string
  status: 'healthy' | 'cooldown' | 'error'
  total_requests: number
  avg_ttft_ms: number
  error_rate: number
  weight: number
  // Token 数据
  total_input_tokens: number
  total_output_tokens: number
  total_cache_read_tokens: number
  total_cache_creation_tokens: number
  cache_hit_rate: number
}

// 请求日志（学 cc-switch RequestLog，完整版）
export interface RequestLog {
  id: string
  timestamp: string
  // 请求信息
  model: string
  provider: string
  app_type: string                     // pi/opencode/claude/codex/...
  key_name: string                      // 用的哪个 key（主密钥/好友名，区分使用者，入库字段）
  session_id: string
  // Token 数据（学 cc-switch）
  input_tokens: number                 // 输入 tokens（原始）
  fresh_input_tokens: number           // 新鲜输入（不含缓存）
  output_tokens: number                // 输出 tokens
  cache_read_tokens: number            // 缓存读取
  cache_creation_tokens: number        // 缓存写入
  cache_hit_rate: number               // 缓存命中率
  real_total_tokens: number            // 真实总 tokens
  // 成本
  // 延迟/速度
  ttft_ms: number                      // 首字时间（毫秒）
  latency_ms: number                   // 总延迟（毫秒）
  duration_s: number                   // 总时长（秒）
  output_speed_tps: number             // 输出速度（tokens/s）
  // 状态
  status_code: number                  // HTTP 状态码
  error: string | null
  is_stream: boolean
}

export interface TrendPoint {
  time: string
  requests: number
  avg_ttft_ms: number
  errors: number
  // Token 数据
  input_tokens: number
  output_tokens: number
  cache_read_tokens: number
}

// Sync 相关（学旧前端 + cc-switch）
export interface Host {
  name: string
  ssh: string
  platform: 'win' | 'unix'
}

export interface AdapterInstance {
  name: string
  host: string
  path: string
}

export interface SyncTarget {
  adapter: string
  instances: AdapterInstance[]
}

export interface HostFull {
  name: string
  host: string           // IP 或 hostname
  port: number           // SSH 端口（默认 22）
  user: string           // SSH 用户名
  auth_type: 'key' | 'password'
  key_path: string       // SSH 密钥路径
  password: string       // 密码（可选）
  ssh_alias: string      // ~/.ssh/config 里的别名
  platform: 'win' | 'unix' | 'macos'
  notes: string          // 备注
}

export interface DailyPoint {
  date: string       // YYYY-MM-DD
  requests: number
  input_tokens: number
  output_tokens: number
  cache_read_tokens: number
  cache_creation_tokens: number
  total_tokens: number
}

// 端点管理（学 sub2api EndpointPopover + apicompat 多协议）
export type EndpointProtocol = 'openai_chat' | 'openai_responses' | 'anthropic_messages' | 'gemini' | 'ollama' | 'custom'

export interface Endpoint {
  id: string
  name: string
  protocol: EndpointProtocol
  path: string           // 端点路径，如 /v1/chat/completions
  enabled: boolean
  description: string
  methods: string[]      // 支持的方法
  streaming: boolean     // 是否支持流式
  auth: 'bearer' | 'x-api-key' | 'none'   // 认证方式
  content_type: string   // application/json
}

export const PROTOCOL_LABELS: Record<EndpointProtocol, string> = {
  openai_chat: 'OpenAI Chat',
  openai_responses: 'OpenAI Responses',
  anthropic_messages: 'Anthropic Messages',
  gemini: 'Gemini',
  ollama: 'Ollama',
  custom: 'Custom',
}

// 部署/水平扩展（学 new-api 主从 + Redis 多实例）
// 部署/主备高可用（学 myapi-nginx：客户端固定入口 + Nginx 多 upstream 故障切换）
export interface DeployInstance {
  id: string
  name: string            // 实例名
  role: 'master' | 'backup'   // 主/备（Nginx upstream backup）
  address: string         // 实例地址（host:port）
  region: string          // 位置（手机/win11/pve）
  status: 'running' | 'stopped' | 'degraded'
  cpu: number             // CPU 使用率 %
  mem: number             // 内存使用率 %
  requests: number        // 请求数
  heartbeatMs: number     // 心跳间隔（Nginx 健康检查）
  sqlitePath: string      // SQLite 文件路径（本地库）
}

export interface GatewayEntry {
  listen: number          // 客户端认的固定入口（Nginx listen）
  readTimeout: number     // proxy_read_timeout
  nextUpstream: string    // proxy_next_upstream 故障切换策略
}

export interface SqliteSync {
  enabled: boolean
  sourcePath: string      // 主节点 SQLite 路径
  targetHost: string      // 备用节点主机（scp 同步目标）
  targetPath: string      // 备用节点 SQLite 路径
  intervalSec: number     // 同步间隔（秒）
  lastSync: string        // 上次同步时间
}

// API 密钥管理（学 new-api Token：给各 agent 发 key）
export interface ApiToken {
  id: string
  name: string            // 用途名（如 pi / opencode / codex）
  key: string             // 密钥（sk-xxx）
  status: 'enabled' | 'disabled'
  unlimited_quota: boolean
  remain_quota: number    // 剩余额度（tokens）
  expired_time: string    // 过期时间（'' = 永不过期）
  models: string[]        // 允许的模型（空=全部）
  ip_limit: string        // IP 限制（空=不限）
  created_at: string
  last_used_at: string    // 最后使用
  total_requests: number  // 累计请求
}

// 代理池（学 9router proxyPools：多个代理 URL + 选择策略 + 健康状态）
export interface ProxyPool {
  id: string
  name: string              // 池名（如 家里 / 服务器）
  proxyUrl: string          // 代理 URL
  type: 'http' | 'socks5'
  isActive: boolean
  strictProxy: boolean      // 严格模式（池必须保证）
  testStatus: 'unknown' | 'ok' | 'fail'
}

export const PROXY_STRATEGIES: Array<{ id: 'round-robin' | 'random' | 'none'; label: string }> = [
  { id: 'round-robin', label: '轮流（round-robin）' },
  { id: 'random', label: '随机（random）' },
  { id: 'none', label: '固定第一个（none）' },
]

// 模型级路由规则（调用变体，学 LiteLLM deployment + 9router combo）
// 同一模型可多个 route：不同 key（可空=不带 key）/不同代理（IPv4/IPv6）/独立额度
export interface ModelRoute {
  id: string
  provider_name: string   // 供应商
  channel_name: string    // 通道名（定位到 Provider.channels 里的具体通道）
  real_name: string       // 上游真实模型名（= 通道里某条映射的 name）
  weight: number          // 路由权重（权重高调用多）
}

// MCP 接口（搜索/识图作为 MCP 工具暴露给 agent，学 9router mcp_tools）
export type McpToolType = 'search' | 'vision'

export interface McpToolConfig {
  tool: McpToolType        // search / vision
  enabled: boolean
  models: string[]         // 绑定哪些模型（webSearch 模型→search；vision 模型→vision）
  max_results: number      // search: 最大结果数
  search_type: string      // search: web/news/x
}

export interface McpServerConfig {
  id: string
  name: string             // MCP server 名（如 myapi-search）
  command: string          // 启动命令（如 npx / 可执行路径）
  args: string             // 参数
  env: Record<string, string>
  transport: 'stdio' | 'http' | 'sse'
  url: string              // http/sse 端点
  enabled: boolean
}

// ==================== 路由策略（学 LiteLLM strategy + cc-switch 三态熔断器）====================

// 路由策略（同模型多通道怎么选）
export type RoutingStrategy = 'latency-based' | 'weight' | 'simple-shuffle' | 'least-busy'

// 黏性路由粒度（省 token：路由到一致通道→上游 prompt cache 命中）
export type StickyGranularity = 'session' | 'api-key' | 'continuity'

// 熔断器三态（学 cc-switch CircuitState）
export type CircuitState = 'closed' | 'open' | 'half_open'

// 全挂兜底策略
export type WhenAllUnhealthy = 'keep-all' | 'fail'

// 路由策略配置（后端读取，前端可配）
export interface RoutingConfig {
  strategy: RoutingStrategy
  latency_window_seconds: number       // TTFT 滑动窗口（后端当前按最近 10 个样本统计，秒值暂仅存档）
  sticky: {
    enabled: boolean
    granularity: StickyGranularity     // 黏性粒度：session/api-key/continuity（后端已生效）
    ttl_seconds: number                // 会话亲和 TTL
  }
  circuit_breaker: {
    failure_threshold: number          // 连续失败阈值（建议 3-10）
    error_rate_threshold: number       // 错误率阈值 %
    min_requests: number               // 错误率有意义的最小请求数
    recovery_timeout_seconds: number   // open→half_open 恢复等待（建议 30-120）
    recovery_success_threshold: number // half_open→closed 恢复成功阈值
  }
  timeout: {
    streaming_first_byte: number       // 流式首字超时（秒）
    streaming_idle: number             // 流式空闲超时（秒）——已存储，后端空闲超时逻辑待实现
    non_streaming: number              // 非流式超时（秒）
  }
  when_all_unhealthy: WhenAllUnhealthy
  auto_mode: {
    enabled: boolean                   // 全自动 Auto 路由（未来）
    capability_match: boolean          // capability 匹配（硬约束：带图/带思考必满足，不可关闭；字段保留兼容）
  }
}

export const ROUTING_STRATEGIES: Array<{ id: RoutingStrategy; label: string }> = [
  { id: 'latency-based', label: '延迟感知（TTFT 最低优先）' },
  { id: 'weight', label: '权重（按通道权重随机）' },
  { id: 'simple-shuffle', label: '随机（简单洗牌）' },
  { id: 'least-busy', label: '最闲（并发最低）' },
]

export const STICKY_GRANULARITIES: Array<{ id: StickyGranularity; label: string }> = [
  { id: 'session', label: '会话亲和（session_id）' },
  { id: 'api-key', label: 'API-key 亲和（key hash）' },
  { id: 'continuity', label: '连续性（previous_response_id）' },
]

export const WHEN_ALL_UNHEALTHY_OPTIONS: Array<{ id: WhenAllUnhealthy; label: string }> = [
  { id: 'keep-all', label: '降级继续（keep-all，全挂也用）' },
  { id: 'fail', label: '直接失败（fail）' },
]

export const DEFAULT_ROUTING_CONFIG: RoutingConfig = {
  strategy: 'latency-based',
  latency_window_seconds: 60,
  sticky: { enabled: true, granularity: 'session', ttl_seconds: 300 },
  circuit_breaker: {
    failure_threshold: 5,
    error_rate_threshold: 50,
    min_requests: 5,
    recovery_timeout_seconds: 30,
    recovery_success_threshold: 3,
  },
  timeout: { streaming_first_byte: 30, streaming_idle: 60, non_streaming: 120 },
  when_all_unhealthy: 'keep-all',
  auto_mode: { enabled: true, capability_match: true },
}
