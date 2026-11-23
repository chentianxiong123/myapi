import { X, Copy } from 'lucide-react'
import type { RequestLog } from '@/lib/types'

interface Props {
  log: RequestLog | null
  onClose: () => void
}

function formatTokens(n: number) {
  return n.toLocaleString()
}

function formatCost(n: number) {
  return `$${n.toFixed(6)}`
}

export default function RequestDetailPanel({ log, onClose }: Props) {
  if (!log) return null

  const rows: Array<{ label: string; value: string; mono?: boolean }> = [
    { label: 'Request ID', value: log.id, mono: true },
    { label: '时间', value: new Date(log.timestamp).toLocaleString('zh-CN') },
    { label: '模型', value: log.model, mono: true },
    { label: 'Provider', value: log.provider },
    { label: '应用', value: log.app_type },
    { label: '会话 ID', value: log.session_id, mono: true },
    { label: '', value: '' },
    { label: '输入 Tokens（新鲜）', value: formatTokens(log.fresh_input_tokens) },
    { label: '输入 Tokens（原始）', value: formatTokens(log.input_tokens) },
    { label: '输出 Tokens', value: formatTokens(log.output_tokens) },
    { label: '缓存读取', value: formatTokens(log.cache_read_tokens) },
    { label: '缓存写入', value: formatTokens(log.cache_creation_tokens) },
    { label: '缓存命中率', value: `${(log.cache_hit_rate * 100).toFixed(1)}%` },
    { label: '真实总 Tokens', value: formatTokens(log.real_total_tokens) },
    { label: '', value: '' },
    { label: '首字时间', value: `${log.ttft_ms}ms` },
    { label: '总时长', value: `${log.duration_s.toFixed(1)}s` },
    { label: '输出速度', value: `${log.output_speed_tps.toFixed(0)} tok/s` },
    { label: '', value: '' },
    { label: '成本', value: formatCost(log.cost) },
    { label: '状态码', value: String(log.status_code) },
    { label: '流式', value: log.is_stream ? '是' : '否' },
    ...(log.error ? [{ label: '错误', value: log.error }] : []),
  ]

  return (
    <div className="fixed inset-y-0 right-0 z-40 w-96 overflow-auto border-l border-gray-200 bg-white shadow-xl dark:border-gray-700 dark:bg-gray-800">
      <div className="sticky top-0 flex items-center justify-between border-b border-gray-200 bg-white px-4 py-3 dark:border-gray-700 dark:bg-gray-800">
        <h3 className="text-sm font-semibold">请求详情</h3>
        <button onClick={onClose} className="rounded p-1 hover:bg-gray-100 dark:hover:bg-gray-700">
          <X className="h-4 w-4" />
        </button>
      </div>
      <div className="p-4">
        {rows.map((row, i) => (
          row.label ? (
            <div key={i} className="mb-3">
              <div className="text-xs text-gray-400">{row.label}</div>
              <div className={`mt-0.5 text-sm ${row.mono ? 'font-mono' : ''} flex items-center gap-1`}>
                <span className="break-all">{row.value}</span>
                {row.mono && (
                  <button onClick={() => navigator.clipboard.writeText(row.value)} className="text-gray-300 hover:text-gray-500">
                    <Copy className="h-3 w-3" />
                  </button>
                )}
              </div>
            </div>
          ) : (
            <hr key={i} className="my-3 border-gray-100 dark:border-gray-700" />
          )
        ))}
      </div>
    </div>
  )
}
