import { useMemo } from 'react'

const WEEKS = 53

interface HeatmapProps {
  data: Array<{ date: string; requests: number; tokens: number }>
  loading?: boolean
}

function dayKey(date: Date): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`
}

function addDays(date: Date, days: number): Date {
  const d = new Date(date)
  d.setDate(d.getDate() + days)
  return d
}

function levelOf(value: number, max: number): number {
  if (value === 0) return 0
  const ratio = value / max
  if (ratio < 0.25) return 1
  if (ratio < 0.5) return 2
  if (ratio < 0.75) return 3
  return 4
}

const HEAT_COLORS = [
  'bg-gray-100 dark:bg-gray-800',
  'bg-blue-100 dark:bg-blue-900/40',
  'bg-blue-200 dark:bg-blue-800/60',
  'bg-blue-400 dark:bg-blue-600',
  'bg-blue-600 dark:bg-blue-400',
]

export default function Heatmap({ data, loading }: HeatmapProps) {
  const byDay = useMemo(() => {
    const map = new Map<string, { requests: number; tokens: number }>()
    data.forEach(d => map.set(d.date, { requests: d.requests, tokens: d.tokens }))
    return map
  }, [data])

  const maxRequests = useMemo(() => Math.max(...data.map(d => d.requests), 1), [data])

  const weeks = useMemo(() => {
    const today = new Date()
    const weekday = (today.getDay() + 6) % 7
    const thisMonday = addDays(today, -weekday)
    const firstDay = addDays(thisMonday, -(WEEKS - 1) * 7)

    const result: Array<Array<{ date: Date; count: number; future: boolean }>> = []
    let current = new Date(firstDay)
    for (let w = 0; w < WEEKS; w++) {
      const week: Array<{ date: Date; count: number; future: boolean }> = []
      for (let d = 0; d < 7; d++) {
        const date = new Date(current)
        const isFuture = date.getTime() > today.getTime()
        const stat = byDay.get(dayKey(date))
        week.push({ date: new Date(date), count: stat?.requests || 0, future: isFuture })
        current = addDays(current, 1)
      }
      result.push(week)
    }
    return result
  }, [byDay])

  if (loading) {
    return <div className="h-32 animate-pulse rounded-xl bg-gray-200 dark:bg-gray-700" />
  }

  return (
    <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
      <div className="mb-4 flex items-center justify-between">
        <h3 className="text-sm font-semibold">使用热力图（过去 53 周）</h3>
        <div className="flex items-center gap-1 text-xs text-gray-400">
          <span>少</span>
          {HEAT_COLORS.map((c, i) => (
            <div key={i} className={`h-3 w-3 rounded-sm ${c}`} />
          ))}
          <span>多</span>
        </div>
      </div>

      {/* 月份标签 */}
      <div className="mb-1 flex gap-[3px] pl-8">
        {weeks.map((week, wi) => {
          const firstDay = week[0]?.date
          const showMonth = wi === 0 || (firstDay && firstDay.getMonth() !== weeks[wi - 1][0].date.getMonth())
          return (
            <div key={wi} className="w-3 text-[9px] text-gray-400">
              {showMonth && firstDay ? `${firstDay.getMonth() + 1}月` : ''}
            </div>
          )
        })}
      </div>

      {/* 热力图格子 */}
      <div className="flex gap-[3px]">
        <div className="flex flex-col gap-[3px] text-[9px] text-gray-400">
          <div className="h-3" />
          {['一', '', '三', '', '五', '', '日'].map((d, i) => (
            <div key={i} className="h-3">{d}</div>
          ))}
        </div>
        {weeks.map((week, wi) => (
          <div key={wi} className="flex flex-col gap-[3px]">
            {week.map((day, di) => {
              const level = day.future ? -1 : levelOf(day.count, maxRequests)
              return (
                <div
                  key={di}
                  className={`h-3 w-3 rounded-sm ${day.future ? 'opacity-0' : HEAT_COLORS[level + 1]}`}
                  title={day.future ? '' : `${day.date.toLocaleDateString('zh-CN')}: ${day.count} 请求`}
                />
              )
            })}
          </div>
        ))}
      </div>
    </div>
  )
}
