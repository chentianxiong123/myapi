// Provider（学cpa 的 openai-compatibility 结构）
export interface Provider {
  name: string
  base_url: string
  disabled: boolean
  headers: Record<string, string>
  keys: Array<{ 'api-key': string }>
  models: Array<{ name: string; alias?: string }>
}

// 对外暴露的模型（按 alias 分组）
export interface ExposedModel {
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
