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
