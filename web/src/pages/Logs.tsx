import { useEffect, useState } from 'react'
import { RefreshCw, Filter, Download } from 'lucide-react'
import type { RequestLog } from '@/lib/types'
import { exportCSV, exportJSON } from '@/lib/export'
import RequestLogTable from '@/components/RequestLogTable'

export default function Logs() {
  const [logs, setLogs] = useState<RequestLog[]>([])
  const [loading, setLoading] = useState(true)
  const [refreshing, setRefreshing] = useState(false)
  const [filterModel, setFilterModel] = useState('')
  const [filterProvider, setFilterProvider] = useState('')
  const [filterError, setFilterError] = useState<'all' | 'error' | 'success'>('all')
  const [limit, setLimit] = useState(100)

  useEffect(() => { loadLogs() }, [limit])

  async function loadLogs() {
    setRefreshing(true)
    try {
      const res = await fetch(`/api/logs?limit=${limit}`)
      const data = await res.json()
      setLogs(data.logs || [])
    } catch {
      setLogs(generateMockLogs(limit))
    } finally {
      setLoading(false)
      setRefreshing(false)
    }
  }

  const filtered = logs.filter(log => {
    if (filterModel && !log.model.includes(filterModel)) return false
    if (filterProvider && !log.provider.includes(filterProvider)) return false
    if (filterError === 'error' && !log.error) return false
    if (filterError === 'success' && log.error) return false
    return true
  })

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold">通用日志</h2>
        <div className="flex gap-2">
          <button onClick={() => exportCSV(filtered, 'request-logs')}
            className="flex items-center gap-1.5 rounded-lg border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700">
            <Download className="h-4 w-4" />导出
          </button>
          <button onClick={loadLogs} disabled={refreshing}
            className="flex items-center gap-1.5 rounded-lg border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700 disabled:opacity-50">
            <RefreshCw className={`h-4 w-4 ${refreshing ? 'animate-spin' : ''}`} />刷新
          </button>
        </div>
      </div>

      {/* 筛选栏（学 new-api 通用日志） */}
      <div className="flex flex-wrap items-center gap-3 rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-800">
        <Filter className="h-4 w-4 text-gray-400" />
        <input value={filterModel} onChange={e => setFilterModel(e.target.value)} placeholder="模型名称..."
          className="w-44 rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700" />
        <input value={filterProvider} onChange={e => setFilterProvider(e.target.value)} placeholder="渠道..."
          className="w-36 rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700" />
        <select value={filterError} onChange={e => setFilterError(e.target.value as any)}
          className="rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700">
          <option value="all">所有类型</option>
          <option value="success">成功</option>
          <option value="error">错误</option>
        </select>
        <select value={limit} onChange={e => setLimit(parseInt(e.target.value))}
          className="rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700">
          <option value={50}>50 条</option>
          <option value={100}>100 条</option>
          <option value={500}>500 条</option>
          <option value={1000}>1000 条</option>
        </select>
        <span className="ml-auto text-xs text-gray-400">显示 {filtered.length} / {logs.length} 条</span>
      </div>

      <RequestLogTable logs={filtered} loading={loading} />
    </div>
  )
}

function generateMockLogs(count: number): RequestLog[] {
  const models = ['deepseek-v4-flash', 'agnes-3.0-flash', 'glm-5.3', 'kimi-k3.1-pro', 'gemini-3.5-flash-lite', 'grok-4.7', 'hy4', 'auto']
  const providers = ['SenseNova', 'pie-xian', 'iamhc', 'agnes']
  const apps = ['pi', 'opencode', 'claude', 'codex']
  const errors = [null, null, null, null, null, 'timeout', null, null, '502 Bad Gateway', null, null, null]

  return Array.from({ length: count }, (_, i) => {
    const error = errors[i % errors.length]
    const input_tokens = Math.floor(Math.random() * 3000) + 100
    const cache_read = Math.floor(Math.random() * 2000)
    const cache_creation = Math.floor(Math.random() * 500)
    const output_tokens = Math.floor(Math.random() * 800) + 50
    const latency = Math.floor(Math.random() * 2000) + 500
    const duration = latency / 1000
    return {
      id: `req-${i}`,
      timestamp: new Date(Date.now() - i * 45000).toISOString(),
      model: models[i % models.length],
      provider: providers[i % providers.length],
      app_type: apps[i % apps.length],
      session_id: `session-${Math.floor(i / 3)}`,
      input_tokens,
      fresh_input_tokens: input_tokens - cache_read,
      output_tokens,
      cache_read_tokens: cache_read,
      cache_creation_tokens: cache_creation,
      cache_hit_rate: (cache_read / (input_tokens + cache_creation)) || 0,
      real_total_tokens: input_tokens + output_tokens + cache_creation + cache_read,
      ttft_ms: error ? 0 : Math.floor(Math.random() * 300) + 100,
      latency_ms: latency,
      duration_s: duration,
      output_speed_tps: output_tokens / duration,
      status_code: error ? 502 : 200,
      error,
      is_stream: true,
    }
  })
}
