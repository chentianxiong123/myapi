import { useState, useRef, useEffect } from 'react'
import { Calendar, ChevronDown, ChevronLeft, ChevronRight } from 'lucide-react'

export type RangePreset = 'today' | '1d' | '7d' | '14d' | '30d' | 'all' | 'custom'

export interface RangeSelection {
  preset: RangePreset
  customStart?: Date
  customEnd?: Date
}

const PRESETS: Array<{ value: RangePreset; label: string }> = [
  { value: 'today', label: '今天' },
  { value: '1d', label: '昨天' },
  { value: '7d', label: '7 天' },
  { value: '14d', label: '14 天' },
  { value: '30d', label: '30 天' },
  { value: 'all', label: '全部' },
  { value: 'custom', label: '自定义' },
]

export function getRangeLabel(sel: RangeSelection): string {
  if (sel.preset === 'custom' && sel.customStart && sel.customEnd) {
    const fmt = (d: Date) => `${d.getMonth() + 1}/${d.getDate()}`
    return `${fmt(sel.customStart)} - ${fmt(sel.customEnd)}`
  }
  return PRESETS.find(p => p.value === sel.preset)?.label || '选择范围'
}

export function getRangeDates(sel: RangeSelection): { start: Date; end: Date } {
  const now = new Date()
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate())
  switch (sel.preset) {
    case 'today': return { start: today, end: now }
    case '1d': { const y = new Date(today); y.setDate(y.getDate() - 1); return { start: y, end: today } }
    case '7d': { const s = new Date(today); s.setDate(s.getDate() - 7); return { start: s, end: now } }
    case '14d': { const s = new Date(today); s.setDate(s.getDate() - 14); return { start: s, end: now } }
    case '30d': { const s = new Date(today); s.setDate(s.getDate() - 30); return { start: s, end: now } }
    case 'all': return { start: new Date(2020, 0, 1), end: now }
    case 'custom': return { start: sel.customStart || today, end: sel.customEnd || now }
  }
}

export default function DateRangePicker({
  selection,
  onChange,
}: {
  selection: RangeSelection
  onChange: (sel: RangeSelection) => void
}) {
  const [open, setOpen] = useState(false)
  const [displayMonth, setDisplayMonth] = useState(new Date())
  const [selectingStart, setSelectingStart] = useState(true)
  const [draftStart, setDraftStart] = useState<Date | null>(null)
  const [datetimeStart, setDatetimeStart] = useState('')
  const [datetimeEnd, setDatetimeEnd] = useState('')
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    function handleClick(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', handleClick)
    return () => document.removeEventListener('mousedown', handleClick)
  }, [])

  function getCalendarDays(month: Date): (Date | null)[] {
    const first = new Date(month.getFullYear(), month.getMonth(), 1)
    const last = new Date(month.getFullYear(), month.getMonth() + 1, 0)
    const startDay = first.getDay()
    const days: (Date | null)[] = []
    for (let i = 0; i < startDay; i++) days.push(null)
    for (let d = 1; d <= last.getDate(); d++) {
      days.push(new Date(month.getFullYear(), month.getMonth(), d))
    }
    return days
  }

  function selectDate(date: Date) {
    if (selectingStart) {
      setDraftStart(date)
      setSelectingStart(false)
    } else {
      const start = draftStart || date
      const end = date < start ? start : date
      onChange({ preset: 'custom', customStart: start, customEnd: end })
      setOpen(false)
      setSelectingStart(true)
    }
  }

  const weekdays = ['日', '一', '二', '三', '四', '五', '六']
  const calendarDays = getCalendarDays(displayMonth)
  const today = new Date()

  // datetime-local 辅助（学 new-api start_time/end_time）
  function toDatetimeLocal(d: Date): string {
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}T${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
  }
  function fromDatetimeLocal(v: string): Date | null {
    if (!v) return null
    const d = new Date(v)
    return isNaN(d.getTime()) ? null : d
  }
  function applyDatetime() {
    const s = fromDatetimeLocal(datetimeStart)
    const e = fromDatetimeLocal(datetimeEnd)
    if (s && e) {
      onChange({ preset: 'custom', customStart: s, customEnd: e })
      setOpen(false)
    }
  }

  return (
    <div className="relative" ref={ref}>
      <button
        onClick={() => setOpen(!open)}
        className="flex items-center gap-2 rounded-lg border border-gray-300 px-3 py-1.5 text-sm dark:border-gray-600 dark:bg-gray-700"
      >
        <Calendar className="h-4 w-4 text-gray-400" />
        <span>{getRangeLabel(selection)}</span>
        <ChevronDown className="h-3 w-3 text-gray-400" />
      </button>

      {open && (
        <div className="absolute left-0 top-full z-30 mt-1 w-72 rounded-xl border border-gray-200 bg-white p-3 shadow-lg dark:border-gray-700 dark:bg-gray-800">
          {/* Presets */}
          <div className="mb-3 flex flex-wrap gap-1">
            {PRESETS.filter(p => p.value !== 'custom').map(p => (
              <button
                key={p.value}
                onClick={() => { onChange({ preset: p.value }); setOpen(false) }}
                className={`rounded px-2 py-1 text-xs ${
                  selection.preset === p.value
                    ? 'bg-blue-600 text-white'
                    : 'bg-gray-100 text-gray-600 hover:bg-gray-200 dark:bg-gray-700 dark:text-gray-300'
                }`}
              >
                {p.label}
              </button>
            ))}
          </div>

          {/* 精确起止时间（学 new-api datetime-local start_time/end_time） */}
          <div className="mb-3 border-t border-gray-200 pt-3 dark:border-gray-700">
            <div className="mb-1.5 text-xs font-medium text-gray-500">精确时段</div>
            <div className="flex items-center gap-1.5">
              <input type="datetime-local" value={datetimeStart} onChange={e => setDatetimeStart(e.target.value)}
                className="w-full rounded border border-gray-300 px-1.5 py-1 text-xs dark:border-gray-600 dark:bg-gray-700" />
              <span className="text-xs text-gray-400">至</span>
              <input type="datetime-local" value={datetimeEnd} onChange={e => setDatetimeEnd(e.target.value)}
                className="w-full rounded border border-gray-300 px-1.5 py-1 text-xs dark:border-gray-600 dark:bg-gray-700" />
            </div>
            <button onClick={applyDatetime}
              className="mt-1.5 w-full rounded bg-blue-600 py-1 text-xs text-white hover:bg-blue-700">
              应用
            </button>
          </div>

          {/* Custom calendar */}
          <div className="border-t border-gray-200 pt-3 dark:border-gray-700">
            <div className="mb-2 flex items-center justify-between">
              <button onClick={() => setDisplayMonth(new Date(displayMonth.getFullYear(), displayMonth.getMonth() - 1))}
                className="rounded p-1 hover:bg-gray-100 dark:hover:bg-gray-700">
                <ChevronLeft className="h-4 w-4" />
              </button>
              <span className="text-sm font-medium">
                {displayMonth.getFullYear()} 年 {displayMonth.getMonth() + 1} 月
              </span>
              <button onClick={() => setDisplayMonth(new Date(displayMonth.getFullYear(), displayMonth.getMonth() + 1))}
                className="rounded p-1 hover:bg-gray-100 dark:hover:bg-gray-700">
                <ChevronRight className="h-4 w-4" />
              </button>
            </div>
            <div className="grid grid-cols-7 gap-0.5 text-center">
              {weekdays.map(w => (
                <div key={w} className="py-1 text-xs text-gray-400">{w}</div>
              ))}
              {calendarDays.map((date, i) => {
                if (!date) return <div key={i} />
                const isToday = date.toDateString() === today.toDateString()
                const isSelected = selection.preset === 'custom' &&
                  ((selection.customStart && date.toDateString() === selection.customStart.toDateString()) ||
                   (selection.customEnd && date.toDateString() === selection.customEnd.toDateString()))
                return (
                  <button
                    key={i}
                    onClick={() => selectDate(date)}
                    className={`rounded py-1 text-xs ${
                      isSelected ? 'bg-blue-600 text-white' :
                      isToday ? 'bg-blue-50 font-bold text-blue-600 dark:bg-blue-900/30' :
                      'hover:bg-gray-100 dark:hover:bg-gray-700'
                    }`}
                  >
                    {date.getDate()}
                  </button>
                )
              })}
            </div>
            {selection.preset === 'custom' && (
              <p className="mt-2 text-xs text-gray-400">
                {selectingStart ? '选择开始日期' : '选择结束日期'}
              </p>
            )}
          </div>
        </div>
      )}
    </div>
  )
}
