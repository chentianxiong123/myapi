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

// 统计
export interface StatsSummary {
  total_requests: number
  success_count: number
  error_count: number
  error_rate: number
  avg_ttft_ms: number
  avg_latency_ms: number
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
}

export interface RequestLog {
  timestamp: string
  model: string
  provider: string
  ttft_ms: number
  latency_ms: number
  tokens_in: number
  tokens_out: number
  error: string | null
  session_id: string
}

export interface TrendPoint {
  time: string
  requests: number
  avg_ttft_ms: number
  errors: number
}
