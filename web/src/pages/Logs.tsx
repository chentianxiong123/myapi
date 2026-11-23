import { useEffect, useState } from 'react'
import { RefreshCw, Filter, XCircle, CheckCircle, Download } from "lucide-react"
import { exportJSON, exportCSV } from "@/lib/export"
import type { RequestLog } from '@/lib/types'

const COLUMNS = [
  { id: 'time', label: '时间', visible: true, locked: true },
  { id: 'model', label: '模型', visible: true },
  { id: 'provider', label: 'Provider', visible: true },
  { id: 'ttft', label: '首字', visible: true },
  { id: 'input', label: '输入', visible: true },
  { id: 'output', label: '输出', visible: true },
  { id: 'cache_read', label: '缓存读', visible: true },
  { id: 'cache_write', label: '缓存写', visible: false },
  { id: 'hit_rate', label: '命中率', visible: false },
  { id: 'cost', label: '成本', visible: true },
  { id: 'speed', label: '速度', visible: false },
  { id: 'duration', label: '时长', visible: false },
  { id: 'status_code', label: '状态码', visible: false },
  { id: 'error', label: '错误', visible: true },
]

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`
  return n.toString()
}

function formatCost(n: number): string {
  if (n >= 0.01) return `$${n.toFixed(4)}`
  return `$${n.toFixed(6)}`
}

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

  function formatTime(ts: string) {
    const d = new Date(ts)
    const now = new Date()
    const clock = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}:${String(d.getSeconds()).padStart(2, '0')}`
    if (d.toDateString() === now.toDateString()) return clock
    return `${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')} ${clock}`
  }

  const visibleCols = COLUMNS.filter(c => c.visible)

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold">请求日志</h2>
        <button onClick={() => exportCSV(filtered, "request-logs")} className="flex items-center gap-1.5 rounded-lg border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700"><Download className="h-4 w-4" />导出 CSV</button>
          <button onClick={() => exportJSON(filtered, "request-logs")} className="flex items-center gap-1.5 rounded-lg border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700"><Download className="h-4 w-4" />导出 JSON</button>
          <button onClick={loadLogs} disabled={refreshing}
          className="flex items-center gap-1.5 rounded-lg border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700 disabled:opacity-50">
          <RefreshCw className={`h-4 w-4 ${refreshing ? 'animate-spin' : ''}`} />刷新
        </button>
      </div>

      {/* 筛选栏 */}
      <div className="flex flex-wrap items-center gap-3 rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-800">
        <Filter className="h-4 w-4 text-gray-400" />
        <input value={filterModel} onChange={e => setFilterModel(e.target.value)} placeholder="筛选模型..."
          className="w-48 rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700" />
        <input value={filterProvider} onChange={e => setFilterProvider(e.target.value)} placeholder="筛选 Provider..."
          className="w-40 rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700" />
        <select value={filterError} onChange={e => setFilterError(e.target.value as any)}
          className="rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700">
          <option value="all">全部状态</option>
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

      {/* 表格 */}
      <div className="overflow-x-auto rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
        <table className="w-full text-left text-sm">
          <thead>
            <tr className="border-b border-gray-200 dark:border-gray-700">
              {visibleCols.map(col => (
                <th key={col.id} className="whitespace-nowrap px-3 py-3 font-medium text-gray-500 dark:text-gray-400">{col.label}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {loading ? (
              <tr><td colSpan={visibleCols.length} className="px-4 py-8 text-center text-gray-400">加载中…</td></tr>
            ) : filtered.length === 0 ? (
              <tr><td colSpan={visibleCols.length} className="px-4 py-8 text-center text-gray-400">没有匹配的日志</td></tr>
            ) : filtered.map((log, i) => (
              <tr key={i} className="border-b border-gray-100 hover:bg-gray-50 dark:border-gray-800 dark:hover:bg-gray-700/50">
                <td className="whitespace-nowrap px-3 py-2 font-mono text-xs text-gray-500">{formatTime(log.timestamp)}</td>
                <td className="whitespace-nowrap px-3 py-2 font-mono text-xs">{log.model}</td>
                <td className="whitespace-nowrap px-3 py-2">{log.provider}</td>
                <td className="whitespace-nowrap px-3 py-2 font-mono text-xs">{log.ttft_ms > 0 ? `${log.ttft_ms}ms` : '-'}</td>
                <td className="whitespace-nowrap px-3 py-2 font-mono text-xs">{formatTokens(log.fresh_input_tokens)}</td>
                <td className="whitespace-nowrap px-3 py-2 font-mono text-xs">{formatTokens(log.output_tokens)}</td>
                <td className="whitespace-nowrap px-3 py-2 font-mono text-xs text-purple-600">{formatTokens(log.cache_read_tokens)}</td>
                <td className="whitespace-nowrap px-3 py-2 font-mono text-xs text-indigo-600">{formatTokens(log.cache_creation_tokens)}</td>
                <td className="whitespace-nowrap px-3 py-2 font-mono text-xs">{formatTokens(log.cache_hit_rate * 100)}%</td>
                <td className="whitespace-nowrap px-3 py-2 font-mono text-xs">{formatCost(log.cost)}</td>
                <td className="whitespace-nowrap px-3 py-2 font-mono text-xs">{log.output_speed_tps.toFixed(0)} t/s</td>
                <td className="whitespace-nowrap px-3 py-2 font-mono text-xs">{log.duration_s.toFixed(1)}s</td>
                <td className="whitespace-nowrap px-3 py-2 font-mono text-xs">{log.status_code}</td>
                <td className="whitespace-nowrap px-3 py-2">
                  {log.error ? (
                    <span className="flex items-center gap-1 text-red-500"><XCircle className="h-3.5 w-3.5" /><span className="text-xs">{log.error}</span></span>
                  ) : (
                    <CheckCircle className="h-3.5 w-3.5 text-green-500" />
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
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
      cost: Math.random() * 0.05 + 0.001,
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
