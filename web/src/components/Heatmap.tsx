import { useMemo, useState } from 'react'

const WEEKS = 53

interface HeatmapProps {
  data: Array<{ date: string; requests: number; tokens: number }>
  loading?: boolean
}

interface HoverState {
  date: Date
  requests: number
  tokens: number
  future: boolean
  x: number
  y: number
}

function dayKey(date: Date): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`
}

function addDays(date: Date, days: number): Date {
  const d = new Date(date)
  d.setDate(d.getDate() + days)
  return d
}

// 学 cc-switch：按非零天的四分位分档，避免个别高峰日把其余格子都压成最浅
function buildThresholds(values: number[]): number[] {
  const sorted = values.filter(v => v > 0).sort((a, b) => a - b)
  if (sorted.length === 0) return [Infinity, Infinity, Infinity]
  const at = (q: number) => sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * q))]
  return [at(0.25), at(0.5), at(0.75)]
}

function levelOf(value: number, thresholds: number[]): number {
  if (value <= 0) return 0
  if (value <= thresholds[0]) return 1
  if (value <= thresholds[1]) return 2
  if (value <= thresholds[2]) return 3
  return 4
}

// GitHub 贡献图绿格配色（5 级，0 = 空）
const HEAT_COLORS = [
  'bg-gray-100 dark:bg-gray-700/50',
  'bg-green-100 dark:bg-green-900/40',
  'bg-green-300 dark:bg-green-800/70',
  'bg-green-500 dark:bg-green-600',
  'bg-green-700 dark:bg-green-500',
]

export default function Heatmap({ data, loading }: HeatmapProps) {
  const [hover, setHover] = useState<HoverState | null>(null)

  const byDay = useMemo(() => {
    const map = new Map<string, { requests: number; tokens: number }>()
    data.forEach(d => map.set(d.date, { requests: d.requests, tokens: d.tokens }))
    return map
  }, [data])

  const thresholds = useMemo(() => buildThresholds(data.map(d => d.requests)), [data])

  const weeks = useMemo(() => {
    const today = new Date()
    const weekday = (today.getDay() + 6) % 7
    const thisMonday = addDays(today, -weekday)
    const firstDay = addDays(thisMonday, -(WEEKS - 1) * 7)

    const result: Array<Array<{ date: Date; count: number; tokens: number; future: boolean }>> = []
    let current = new Date(firstDay)
    for (let w = 0; w < WEEKS; w++) {
      const week: Array<{ date: Date; count: number; tokens: number; future: boolean }> = []
      for (let d = 0; d < 7; d++) {
        const date = new Date(current)
        const isFuture = date.getTime() > today.getTime()
        const stat = byDay.get(dayKey(date))
        week.push({ date: new Date(date), count: stat?.requests || 0, tokens: stat?.tokens || 0, future: isFuture })
        current = addDays(current, 1)
      }
      result.push(week)
    }
    return result
  }, [byDay])

  const totalRequests = useMemo(() => weeks.flat().filter(c => !c.future).reduce((s, c) => s + c.count, 0), [weeks])
  const totalTokens = useMemo(() => weeks.flat().filter(c => !c.future).reduce((s, c) => s + c.tokens, 0), [weeks])

  if (loading) {
    return <div className="h-32 animate-pulse rounded-xl bg-gray-200 dark:bg-gray-700" />
  }

  // 汇总头：日期范围 + 总量
  const firstDay = weeks[0]?.[0]?.date
  const lastDay = weeks[WEEKS - 1]?.[0]?.date
  const fmtDate = (d?: Date) => d ? `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}` : ''

  return (
    <div className="relative rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
      {/* 头部 */}
      <div className="mb-1 flex items-center justify-between">
        <div>
          <h3 className="text-sm font-semibold">使用热力图</h3>
          <p className="text-xs text-gray-400">过去 53 周的每日用量，颜色深浅可直接对比</p>
        </div>
      </div>

      {/* 汇总行：日期范围 + 总量 */}
      <div className="mb-3 flex items-baseline gap-3">
        <span className="text-sm font-semibold tabular-nums">{fmtDate(firstDay)} – {fmtDate(lastDay)}</span>
        <span className="text-xs text-gray-400 tabular-nums">{totalTokens.toLocaleString()} Tokens · {totalRequests.toLocaleString()} 次请求</span>
      </div>

      {/* 月份标签（grid 铺满） */}
      <div className="overflow-x-auto pb-1">
<div className="mb-1 grid min-w-[700px] pl-8" style={{ gridTemplateColumns: `repeat(${WEEKS}, minmax(0, 1fr))`, gap: '3px' }}>
        {weeks.map((week, wi) => {
          const firstDay = week[0]?.date
          const showMonth = wi === 0 || (firstDay && firstDay.getMonth() !== weeks[wi - 1][0].date.getMonth())
          return (
            <div key={wi} className="overflow-visible whitespace-nowrap text-[9px] text-gray-400">
              {showMonth && firstDay ? `${firstDay.getMonth() + 1}月` : ''}
            </div>
          )
        })}
      </div>
      </div>

      {/* 热力图格子（GitHub 贡献图，grid 铺满） */}
      <div className="overflow-x-auto">
<div className="grid min-w-[700px]" style={{ gridTemplateColumns: `max-content repeat(${WEEKS}, minmax(0, 1fr))`, gap: '3px' }}>
        {/* 周标签列 */}
        <div className="grid" style={{ gridTemplateRows: 'repeat(7, minmax(0, 1fr))', gap: '3px' }}>
          {['', '一', '', '三', '', '五', '', '日'].map((d, i) => (
            <div key={i} className="flex items-center text-[9px] leading-none text-gray-400">{d}</div>
          ))}
        </div>
        {/* 53 列格子 */}
        {weeks.map((week, wi) => (
          <div key={wi} className="grid" style={{ gridTemplateRows: 'repeat(7, minmax(0, 1fr))', gap: '3px' }}>
            {week.map((day, di) => {
              const level = day.future ? -1 : levelOf(day.count, thresholds)
              return (
                <div
                  key={di}
                  className={`aspect-square w-full rounded-[3px] ${day.future ? 'opacity-0' : HEAT_COLORS[level]}`}
                  onMouseEnter={e => {
                    if (day.future) return
                    const rect = e.currentTarget.getBoundingClientRect()
                    const parent = e.currentTarget.parentElement?.parentElement?.getBoundingClientRect()
                    setHover({ date: day.date, requests: day.count, tokens: day.tokens, future: false, x: rect.left - (parent?.left || 0), y: rect.top - (parent?.top || 0) })
                  }}
                  onMouseLeave={() => setHover(null)}
                />
              )
            })}
          </div>
        ))}
      </div>
      </div>

      {/* 悬停提示 */}
      {hover && (
        <div className="pointer-events-none absolute z-20 rounded-md bg-gray-900 px-3 py-2 text-xs text-white shadow-lg dark:bg-gray-700"
          style={{ left: hover.x, top: hover.y - 50, transform: 'translateX(-50%)' }}>
          <div className="font-semibold">{hover.date.toLocaleDateString('zh-CN', { year: 'numeric', month: 'long', day: 'numeric', weekday: 'short' })}</div>
          <div className="mt-0.5 text-gray-300">{hover.requests} 次请求 · {hover.tokens.toLocaleString()} Tokens</div>
        </div>
      )}

      {/* 图例 */}
      <div className="mt-2 flex items-center justify-end gap-1 text-xs text-gray-400">
        <span>少</span>
        {HEAT_COLORS.map((c, i) => <div key={i} className={`h-3 w-3 rounded-sm ${c}`} />)}
        <span>多</span>
      </div>
    </div>
  )
}
