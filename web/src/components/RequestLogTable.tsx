import { CheckCircle, XCircle, Layers, Database, Zap, Activity } from 'lucide-react'
import type { RequestLog } from '@/lib/types'
import type { RequestLogColumnId } from '@/lib/requestLogColumns'

interface Props {
  logs: RequestLog[]
  loading: boolean
  /** 要显示的列（按表格顺序）；不传按默认列 */
  columns?: RequestLogColumnId[]
  /** 点击行（打开详情） */
  onRowClick?: (log: RequestLog) => void
  /** 内部滚动最大高度（学 new-api 卡片内部滚动） */
  scrollHeight?: number
}

// 学 new-api getColorByElapsedTime：耗时颜色分级（首字/耗时共用）
function elapsedColor(ms: number): string {
  if (ms <= 0) return 'text-gray-400'
  if (ms < 1000) return 'text-green-600'
  if (ms < 3000) return 'text-yellow-700'
  if (ms < 5000) return 'text-yellow-600'
  if (ms < 10000) return 'text-orange-500'
  return 'text-red-600'
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`
  return n.toString()
}

// 学 new-api timestamp2string：YYYY-MM-DD HH:mm:ss
function formatTime(ts: string): string {
  const d = new Date(ts)
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}:${String(d.getSeconds()).padStart(2, '0')}`
}

const DEFAULT_COLUMNS: RequestLogColumnId[] = [
  'time', 'app', 'provider', 'model', 'stream', 'input', 'output',
  'cache_read', 'speed', 'ttft',
]

/** new-api 风格请求日志表格（统一 Dashboard tab + Logs 页，列可配置） */
export default function RequestLogTable({ logs, loading, columns = DEFAULT_COLUMNS, onRowClick, scrollHeight }: Props) {
  return (
    <div className="overflow-x-auto rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
      <div className="overflow-y-auto" style={scrollHeight ? { maxHeight: scrollHeight } : undefined}>
      <table className="w-full text-left text-sm">
        <thead>
          <tr className="border-b border-gray-200 dark:border-gray-700">
            {columns.map((id) => (
              <th key={id} className="whitespace-nowrap px-3 py-3 font-medium text-gray-500 dark:text-gray-400">
                {COLUMN_LABELS[id]}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {loading ? (
            <tr><td colSpan={columns.length} className="px-4 py-8 text-center text-gray-400">加载中…</td></tr>
          ) : logs.length === 0 ? (
            <tr><td colSpan={columns.length} className="px-4 py-8 text-center text-gray-400">没有日志</td></tr>
          ) : logs.map((log, i) => (
            <tr
              key={i}
              onClick={() => onRowClick?.(log)}
              className={`border-b border-gray-100 hover:bg-gray-50 dark:border-gray-800 dark:hover:bg-gray-700/50 ${onRowClick ? 'cursor-pointer' : ''}`}
            >
              {columns.map((id) => (
                <td key={id} className="whitespace-nowrap px-3 py-2">
                  {renderCell(id, log)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      </div>
    </div>
  )
}

const COLUMN_LABELS: Record<RequestLogColumnId, string> = {
  time: '时间',
  app: '应用',
  key_name: '使用者',
  provider: 'Provider',
  model: '模型',
  stream: '流',
  input: '输入',
  output: '输出',
  cache_read: '缓存命中',
  cache_write: '缓存写入',
  hit_rate: '命中率',
  speed: '速度',
  ttft: '首字',
  duration: '总耗时',
  status_code: '状态码',
}

function renderCell(id: RequestLogColumnId, log: RequestLog) {
  switch (id) {
    case 'time':
      return (
        <div>
          <div className="font-mono text-xs text-gray-500">{formatTime(log.timestamp)}</div>
          <div className="mt-0.5">
            {log.error ? (
              <span className="rounded bg-red-100 px-1 text-[9px] text-red-600 dark:bg-red-900/30">错误</span>
            ) : (
              <span className="rounded bg-blue-100 px-1 text-[9px] text-blue-600 dark:bg-blue-900/30">消耗</span>
            )}
          </div>
        </div>
      )
    case 'app':
      return (
        <div className="flex items-center gap-1.5">
          <span className="flex h-4 w-4 items-center justify-center rounded-full bg-red-100 text-[9px] font-bold text-red-600 dark:bg-red-900/30">
            {log.app_type.slice(0, 1).toUpperCase()}
          </span>
          <span className="text-xs">{log.app_type}</span>
        </div>
      )
    case 'key_name':
      return (
        <div className="flex items-center gap-1">
          <span className="flex h-4 w-4 items-center justify-center rounded-full bg-orange-100 text-[9px] font-bold text-orange-600 dark:bg-orange-900/30">
            {log.key_name?.includes('好友') ? '友' : '主'}
          </span>
          <span className={`text-xs ${log.key_name?.includes('好友') ? 'text-orange-600' : 'text-gray-500'}`}>{log.key_name || '主密钥'}</span>
        </div>
      )
    case 'provider':
      return (
        <div>
          <div className="font-mono text-xs text-purple-600">#{log.status_code > 0 ? '8' : '-'}</div>
          <div className="mt-0.5 text-xs text-gray-400">{log.provider}</div>
        </div>
      )
    case 'model':
      return (
        <div>
          <div className="flex items-center gap-1.5">
            <Layers className="h-3.5 w-3.5 text-blue-500" />
            <span className="font-mono text-xs">{log.model}</span>
          </div>
          <div className="mt-0.5 text-[10px] text-gray-400">会话 {log.session_id.slice(0, 10)}</div>
        </div>
      )
    case 'stream':
      return (
        <div>
          {log.is_stream ? (
            <div className="flex items-center gap-1">
              <Zap className="h-3 w-3 text-blue-500" />
              <span className="rounded bg-blue-100 px-1 text-[9px] text-blue-600 dark:bg-blue-900/30">流</span>
            </div>
          ) : (
            <span className="rounded bg-gray-100 px-1 text-[9px] text-gray-500 dark:bg-gray-700">—</span>
          )}
        </div>
      )
    case 'input':
      return <span className="font-mono text-xs">{formatTokens(log.fresh_input_tokens)}</span>
    case 'output':
      return <span className="font-mono text-xs">{formatTokens(log.output_tokens)}</span>
    case 'cache_read':
      return (
        <div className="font-mono text-xs">
          <div className="flex items-center gap-1">
            <Database className="h-3 w-3 text-purple-500" />
            {formatTokens(log.cache_read_tokens)}
          </div>
          <div className="mt-0.5 text-[10px] text-gray-400">写 {formatTokens(log.cache_creation_tokens)}</div>
        </div>
      )
    case 'cache_write':
      return <span className="font-mono text-xs text-indigo-600">{formatTokens(log.cache_creation_tokens)}</span>
    case 'hit_rate':
      return <span className="font-mono text-xs">{(log.cache_hit_rate * 100).toFixed(1)}%</span>
    case 'speed':
      return <span className="font-mono text-xs text-blue-500">{log.output_speed_tps.toFixed(0)} t/s</span>
    case 'ttft':
      return (
        <div className={`font-mono text-xs ${elapsedColor(log.ttft_ms)}`}>
          {log.ttft_ms > 0 ? `${(log.ttft_ms / 1000).toFixed(1)}s` : '不适用'}
        </div>
      )
    case 'duration':
      return (
        <div className={`font-mono text-xs ${elapsedColor(log.latency_ms)}`}>
          {log.duration_s.toFixed(1)}s{log.is_stream && <span className="ml-1 rounded bg-pink-100 px-1 text-[9px] text-pink-600 dark:bg-pink-900/30">Stream</span>}
        </div>
      )
    case 'status_code':
      return (
        log.error ? (
          <span className="flex items-center gap-1 text-red-500">
            <XCircle className="h-3.5 w-3.5" />
            <span className="text-xs">{log.status_code}</span>
          </span>
        ) : (
          <span className="flex items-center gap-1 text-green-500">
            <CheckCircle className="h-3.5 w-3.5" />
            <span className="text-xs">{log.status_code}</span>
          </span>
        )
      )
  }
}
