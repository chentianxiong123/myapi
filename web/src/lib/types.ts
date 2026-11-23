// Provider（学 myapi 的 openai-compatibility 结构）
export type ModelProtocol = 'openai_chat' | 'openai_responses' | 'anthropic' | 'gemini' | 'ollama'

// Provider（学 myapi 的 openai-compatibility 结构）
export interface Provider {
  name: string
  base_url: string
  disabled: boolean
  headers: Record<string, string>
  keys: Array<{ 'api-key': string }>
  models: Array<{ name: string; alias?: string; protocol?: ModelProtocol }>
  // 渠道级别代理（每个 Provider 可单独走代理；空=直连/全局代理）
  proxy: string
  // 代理池（学 9router：引用 proxyPools 的一个池 + 选择策略；空=不用池）
  proxy_pool_id: string
  proxy_strategy: 'round-robin' | 'random' | 'none'
}

export const MODEL_PROTOCOLS: Array<{ id: ModelProtocol; label: string; path: string }> = [
  { id: 'openai_chat', label: 'OpenAI Chat', path: '/v1/chat/completions' },
  { id: 'openai_responses', label: 'OpenAI Responses', path: '/v1/responses' },
  { id: 'anthropic', label: 'Anthropic Messages', path: '/v1/messages' },
  { id: 'gemini', label: 'Gemini', path: '/v1beta/models' },
  { id: 'ollama', label: 'Ollama', path: '/api/chat' },
]

// 对外暴露的模型（按 alias 分组）
export interface ExposedModel {
  id?: string
  alias: string
  providers: Array<{
    provider_name: string
    real_name: string
    weight: number
  }>
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
