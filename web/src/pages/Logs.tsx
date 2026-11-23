import { useEffect, useState } from 'react'
import { RefreshCw, Filter, ChevronDown, XCircle, CheckCircle } from 'lucide-react'
import type { RequestLog } from '@/lib/types'

// 列定义（学cc-switch，简化版）
const COLUMNS = [
  { id: 'time', label: '时间', visible: true, locked: true },
  { id: 'model', label: '模型', visible: true },
  { id: 'provider', label: 'Provider', visible: true },
  { id: 'ttft', label: '首字', visible: true },
  { id: 'tokens_in', label: '输入', visible: true },
  { id: 'tokens_out', label: '输出', visible: true },
  { id: 'latency', label: '总时长', visible: false },
  { id: 'session', label: '会话', visible: false },
  { id: 'error', label: '错误', visible: true },
] as const

export default function Logs() {
  const [logs, setLogs] = useState<RequestLog[]>([])
  const [loading, setLoading] = useState(true)
  const [refreshing, setRefreshing] = useState(false)
  const [filterModel, setFilterModel] = useState('')
  const [filterProvider, setFilterProvider] = useState('')
  const [filterError, setFilterError] = useState<'all' | 'error' | 'success'>('all')
  const [limit, setLimit] = useState(100)

  useEffect(() => {
    loadLogs()
  }, [limit])

  async function loadLogs() {
    setRefreshing(true)
    try {
      const res = await fetch(`/api/logs?limit=${limit}`)
      const data = await res.json()
      setLogs(data.logs || [])
    } catch {
      // 后端还没写，用假数据
      setLogs(generateMockLogs(limit))
    } finally {
      setLoading(false)
      setRefreshing(false)
    }
  }

  // 筛选
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

  function formatTokens(n: number) {
    if (n >= 1000000) return `${(n / 1000000).toFixed(1)}M`
    if (n >= 1000) return `${(n / 1000).toFixed(1)}k`
    return n.toString()
  }

  return (
    <div className="space-y-4">
      {/* 顶部：标题 + 刷新 */}
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold">请求日志</h2>
        <button
          onClick={loadLogs}
          disabled={refreshing}
          className="flex items-center gap-1.5 rounded-lg border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700 disabled:opacity-50"
        >
          <RefreshCw className={`h-4 w-4 ${refreshing ? 'animate-spin' : ''}`} />
          刷新
        </button>
      </div>

      {/* 筛选栏 */}
      <div className="flex flex-wrap items-center gap-3 rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-800">
        <Filter className="h-4 w-4 text-gray-400" />
        <input
          value={filterModel}
          onChange={e => setFilterModel(e.target.value)}
          placeholder="筛选模型..."
          className="w-48 rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700"
        />
        <input
          value={filterProvider}
          onChange={e => setFilterProvider(e.target.value)}
          placeholder="筛选 Provider..."
          className="w-40 rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700"
        />
        <select
          value={filterError}
          onChange={e => setFilterError(e.target.value as any)}
          className="rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700"
        >
          <option value="all">全部状态</option>
          <option value="success">成功</option>
          <option value="error">错误</option>
        </select>
        <select
          value={limit}
          onChange={e => setLimit(parseInt(e.target.value))}
          className="rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700"
        >
          <option value={50}>50 条</option>
          <option value={100}>100 条</option>
          <option value={500}>500 条</option>
          <option value={1000}>1000 条</option>
        </select>
        <span className="ml-auto text-xs text-gray-400">
          显示 {filtered.length} / {logs.length} 条
        </span>
      </div>

      {/* 表格 */}
      <div className="overflow-x-auto rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
        <table className="w-full text-left text-sm">
          <thead>
            <tr className="border-b border-gray-200 dark:border-gray-700">
              {COLUMNS.filter(c => c.visible).map(col => (
                <th key={col.id} className="whitespace-nowrap px-4 py-3 font-medium text-gray-500 dark:text-gray-400">
                  {col.label}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {loading ? (
              <tr>
                <td colSpan={COLUMNS.filter(c => c.visible).length} className="px-4 py-8 text-center text-gray-400">
                  加载中…
                </td>
              </tr>
            ) : filtered.length === 0 ? (
              <tr>
                <td colSpan={COLUMNS.filter(c => c.visible).length} className="px-4 py-8 text-center text-gray-400">
                  没有匹配的日志
                </td>
              </tr>
            ) : (
              filtered.map((log, i) => (
                <tr key={i} className="border-b border-gray-100 hover:bg-gray-50 dark:border-gray-800 dark:hover:bg-gray-700/50">
                  <td className="whitespace-nowrap px-4 py-2 font-mono text-xs text-gray-500">
                    {formatTime(log.timestamp)}
                  </td>
                  <td className="whitespace-nowrap px-4 py-2 font-mono text-xs">{log.model}</td>
                  <td className="whitespace-nowrap px-4 py-2">{log.provider}</td>
                  <td className="whitespace-nowrap px-4 py-2 font-mono text-xs">
                    {log.ttft_ms > 0 ? `${log.ttft_ms}ms` : '-'}
                  </td>
                  <td className="whitespace-nowrap px-4 py-2 font-mono text-xs text-gray-500">
                    {formatTokens(log.tokens_in)}
                  </td>
                  <td className="whitespace-nowrap px-4 py-2 font-mono text-xs text-gray-500">
                    {formatTokens(log.tokens_out)}
                  </td>
                  <td className="whitespace-nowrap px-4 py-2">
                    {log.error ? (
                      <span className="flex items-center gap-1 text-red-500">
                        <XCircle className="h-3.5 w-3.5" />
                        <span className="text-xs">{log.error}</span>
                      </span>
                    ) : (
                      <CheckCircle className="h-3.5 w-3.5 text-green-500" />
                    )}
                  </td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>

      {/* 底部加载更多 */}
      {filtered.length >= limit && (
        <div className="text-center">
          <button
            onClick={() => setLimit(limit * 2)}
            className="text-sm text-blue-600 hover:text-blue-700"
          >
            加载更多
          </button>
        </div>
      )}
    </div>
  )
}

// 假数据（后端还没写）
function generateMockLogs(count: number): RequestLog[] {
  const models = ['deepseek-v4-flash', 'agnes-3.0-flash', 'glm-5.3', 'kimi-k3.1-pro', 'gemini-3.5-flash-lite', 'grok-4.7', 'hy4', 'auto']
  const providers = ['SenseNova', 'pie-xian', 'iamhc', 'agnes']
  const errors = [null, null, null, null, null, 'timeout', null, null, '502 Bad Gateway', null, null, null]

  return Array.from({ length: count }, (_, i) => {
    const error = errors[i % errors.length]
    return {
      timestamp: new Date(Date.now() - i * 45000).toISOString(),
      model: models[i % models.length],
      provider: providers[i % providers.length],
      ttft_ms: error ? 0 : Math.floor(Math.random() * 300) + 100,
      latency_ms: error ? 0 : Math.floor(Math.random() * 2000) + 500,
      tokens_in: Math.floor(Math.random() * 3000) + 100,
      tokens_out: Math.floor(Math.random() * 800) + 50,
      error,
      session_id: `session-${Math.floor(i / 3)}`,
    }
  })
}
