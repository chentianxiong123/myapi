import { useEffect, useState } from 'react'
import { RefreshCw, Filter, Download, ChevronDown, Check, X, Columns, ListFilter } from 'lucide-react'
import type { RequestLog } from '@/lib/types'
import { exportCSV, exportJSON } from '@/lib/export'
import RequestLogTable from '@/components/RequestLogTable'
import {
  resolveRequestLogColumns, setRequestLogColumn, TOGGLEABLE_REQUEST_LOG_COLUMNS,
  STATUS_CODE_OPTIONS, type RequestLogColumnId,
} from '@/lib/requestLogColumns'

export default function Logs() {
  const [logs, setLogs] = useState<RequestLog[]>([])
  const [loading, setLoading] = useState(true)
  const [refreshing, setRefreshing] = useState(false)
  const [filterModel, setFilterModel] = useState('')
  const [filterProvider, setFilterProvider] = useState('')
  const [filterError, setFilterError] = useState<'all' | 'error' | 'success'>('all')
  const [filterStatus, setFilterStatus] = useState<number | undefined>(undefined)
  const [limit, setLimit] = useState(100)
  const [columns, setColumns] = useState<RequestLogColumnId[]>(() => resolveRequestLogColumns(undefined))
  const [colOverrides, setColOverrides] = useState<Record<string, boolean>>({})
  const [openMenu, setOpenMenu] = useState<'status' | 'columns' | null>(null)
  // 分页（学 sub2api Pagination）
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(50)

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
    if (filterStatus !== undefined && log.status_code !== filterStatus) return false
    return true
  })

  function toggleColumn(id: RequestLogColumnId) {
    const next = setRequestLogColumn(colOverrides, id, !columns.includes(id))
    setColOverrides(next)
    setColumns(resolveRequestLogColumns(next))
  }

  // 分页计算（学 sub2api Pagination）
  const totalPages = Math.max(1, Math.ceil(filtered.length / pageSize))
  const paginated = filtered.slice((page - 1) * pageSize, page * pageSize)

  function visiblePages(): Array<number | string> {
    const pages: Array<number | string> = []
    const maxVisible = 7
    if (totalPages <= maxVisible) {
      for (let i = 1; i <= totalPages; i++) pages.push(i)
    } else {
      pages.push(1)
      const start = Math.max(2, page - 2)
      const end = Math.min(totalPages - 1, page + 2)
      if (start > 2) pages.push('...')
      for (let i = start; i <= end; i++) pages.push(i)
      if (end < totalPages - 1) pages.push('...')
      pages.push(totalPages)
    }
    return pages
  }

  function goToPage(p: number) {
    if (p >= 1 && p <= totalPages && p !== page) setPage(p)
  }

  const statusLabel = filterStatus === undefined ? '全部状态码' : String(filterStatus)
  const shownToggleable = TOGGLEABLE_REQUEST_LOG_COLUMNS.filter(({ id }) => columns.includes(id)).length

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

      {/* 筛选栏（学 new-api 通用日志 + cc-switch 状态码/列选择） */}
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

        {/* 状态码筛选（学 cc-switch STATUS_CODE_OPTIONS） */}
        <div className="relative">
          <button onClick={() => setOpenMenu(openMenu === 'status' ? null : 'status')}
            className="flex items-center gap-1 rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700">
            <ListFilter className="h-3.5 w-3.5" />
            {statusLabel}
            <ChevronDown className="h-3.5 w-3.5" />
          </button>
          {openMenu === 'status' && (
            <>
              <div className="fixed inset-0 z-40" onClick={() => setOpenMenu(null)} />
              <div className="absolute right-0 z-50 mt-1 w-40 rounded-lg border border-gray-200 bg-white py-1 shadow-lg dark:border-gray-600 dark:bg-gray-800">
                <button onClick={() => { setFilterStatus(undefined); setOpenMenu(null) }}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-sm hover:bg-gray-100 dark:hover:bg-gray-700">
                  {filterStatus === undefined ? <Check className="h-3.5 w-3.5" /> : <span className="w-3.5" />}
                  全部状态码
                </button>
                {STATUS_CODE_OPTIONS.map(code => (
                  <button key={code} onClick={() => { setFilterStatus(code); setOpenMenu(null) }}
                    className="flex w-full items-center gap-2 px-3 py-1.5 text-sm hover:bg-gray-100 dark:hover:bg-gray-700">
                    {filterStatus === code ? <Check className="h-3.5 w-3.5" /> : <span className="w-3.5" />}
                    <span className="tabular-nums">{code}</span>
                  </button>
                ))}
              </div>
            </>
          )}
        </div>

        {/* 列选择器（学 cc-switch TOGGLEABLE_REQUEST_LOG_COLUMNS） */}
        <div className="relative">
          <button onClick={() => setOpenMenu(openMenu === 'columns' ? null : 'columns')}
            className="flex items-center gap-1 rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700">
            <Columns className="h-3.5 w-3.5" />
            列 {shownToggleable}/{TOGGLEABLE_REQUEST_LOG_COLUMNS.length}
            <ChevronDown className="h-3.5 w-3.5" />
          </button>
          {openMenu === 'columns' && (
            <>
              <div className="fixed inset-0 z-40" onClick={() => setOpenMenu(null)} />
              <div className="absolute right-0 z-50 mt-1 w-44 rounded-lg border border-gray-200 bg-white py-1 shadow-lg dark:border-gray-600 dark:bg-gray-800">
                {TOGGLEABLE_REQUEST_LOG_COLUMNS.map(({ id, label }) => (
                  <button key={id} onClick={() => toggleColumn(id)}
                    className="flex w-full items-center gap-2 px-3 py-1.5 text-sm hover:bg-gray-100 dark:hover:bg-gray-700">
                    {columns.includes(id) ? <Check className="h-3.5 w-3.5" /> : <span className="w-3.5" />}
                    {label}
                  </button>
                ))}
              </div>
            </>
          )}
        </div>

        <select value={limit} onChange={e => setLimit(parseInt(e.target.value))}
          className="rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700">
          <option value={50}>50 条</option>
          <option value={100}>100 条</option>
          <option value={500}>500 条</option>
          <option value={1000}>1000 条</option>
        </select>
        <span className="ml-auto text-xs text-gray-400">显示 {filtered.length} / {logs.length} 条</span>
      </div>

      {/* 日志卡片（学 new-api chart-card：独立卡片 + 内部滚动 + footer 分页） */}
      <div className="rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
        <RequestLogTable logs={paginated} loading={loading} columns={columns} scrollHeight={560} />

        {/* footer 分页（学 new-api Pagination floated right，全部靠右） */}
        <div className="flex items-center justify-end gap-3 border-t border-gray-200 px-4 py-2.5 dark:border-gray-700">
          <nav className="flex items-center gap-1">
            <button onClick={() => goToPage(page - 1)} disabled={page === 1}
              className="rounded-md border border-gray-300 px-2 py-1 text-xs text-gray-500 hover:bg-gray-50 disabled:cursor-not-allowed disabled:opacity-50 dark:border-gray-600 dark:hover:bg-gray-700">‹</button>
            {visiblePages().map((p, i) => (
              typeof p === 'number' ? (
                <button key={i} onClick={() => goToPage(p)}
                  className={`rounded-md border px-3 py-1 text-xs ${p === page ? 'border-blue-600 bg-blue-50 font-semibold text-blue-600 dark:bg-blue-900/30' : 'border-gray-300 text-gray-600 hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700'}`}>
                  {p}
                </button>
              ) : (
                <span key={i} className="px-1 text-xs text-gray-400">…</span>
              )
            ))}
            <button onClick={() => goToPage(page + 1)} disabled={page === totalPages}
              className="rounded-md border border-gray-300 px-2 py-1 text-xs text-gray-500 hover:bg-gray-50 disabled:cursor-not-allowed disabled:opacity-50 dark:border-gray-600 dark:hover:bg-gray-700">›</button>
          </nav>
          <div className="flex items-center gap-2 text-xs text-gray-400">
            <span>共 {filtered.length} 条</span>
            <select value={pageSize} onChange={e => { setPageSize(parseInt(e.target.value)); setPage(1) }}
              className="rounded border border-gray-300 px-1 py-0.5 text-xs dark:border-gray-600 dark:bg-gray-700">
              <option value={20}>20 条/页</option>
              <option value={50}>50 条/页</option>
              <option value={100}>100 条/页</option>
            </select>
          </div>
        </div>
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
      key_name: i % 4 === 0 ? '好友:老张' : '主密钥',
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
      status_code: error ? [400, 401, 429, 500][i % 4] : 200,
      error,
      is_stream: true,
    }
  })
}
