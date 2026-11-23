import { useState } from 'react'
import { AreaChart, Area, BarChart, Bar, XAxis, YAxis, Tooltip, ResponsiveContainer, Legend } from 'recharts'
import type { TrendPoint } from '@/lib/types'

interface Props {
  data: TrendPoint[]
  loading: boolean
}

type ChartType = 'area' | 'bar'
type Metric = 'requests' | 'tokens' | 'cost'

export default function TrendChart({ data, loading }: Props) {
  const [chartType, setChartType] = useState<ChartType>('area')
  const [metric, setMetric] = useState<Metric>('requests')

  if (loading) {
    return <div className="h-72 animate-pulse rounded-xl bg-gray-200 dark:bg-gray-700" />
  }

  const Chart = chartType === 'bar' ? BarChart : AreaChart

  const metrics = {
    requests: { key: 'requests', label: '请求数', color: '#2563eb' },
    tokens: { key: 'input_tokens', label: 'Tokens', color: '#7c3aed' },
    cost: { key: 'cost', label: '成本', color: '#059669' },
  }
  const m = metrics[metric]

  return (
    <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
      <div className="mb-4 flex items-center justify-between">
        <h3 className="text-sm font-semibold text-gray-700 dark:text-gray-300">趋势</h3>
        <div className="flex gap-2">
          {/* 图表类型切换 */}
          <div className="flex rounded-lg bg-gray-100 p-0.5 dark:bg-gray-700">
            <button
              onClick={() => setChartType('area')}
              className={`rounded px-2 py-0.5 text-xs ${chartType === 'area' ? 'bg-white shadow dark:bg-gray-800' : 'text-gray-500'}`}
            >面积</button>
            <button
              onClick={() => setChartType('bar')}
              className={`rounded px-2 py-0.5 text-xs ${chartType === 'bar' ? 'bg-white shadow dark:bg-gray-800' : 'text-gray-500'}`}
            >柱状</button>
          </div>
          {/* 指标切换 */}
          <div className="flex rounded-lg bg-gray-100 p-0.5 dark:bg-gray-700">
            {Object.entries(metrics).map(([key, v]) => (
              <button
                key={key}
                onClick={() => setMetric(key as Metric)}
                className={`rounded px-2 py-0.5 text-xs ${metric === key ? 'bg-white shadow dark:bg-gray-800' : 'text-gray-500'}`}
              >{v.label}</button>
            ))}
          </div>
        </div>
      </div>
      <ResponsiveContainer width="100%" height={280}>
        <Chart data={data}>
          <defs>
            <linearGradient id="colorMetric" x1="0" y1="0" x2="0" y2="1">
              <stop offset="5%" stopColor={m.color} stopOpacity={0.3} />
              <stop offset="95%" stopColor={m.color} stopOpacity={0} />
            </linearGradient>
          </defs>
          <XAxis dataKey="time" tick={{ fontSize: 11 }} tickLine={false} axisLine={false} />
          <YAxis tick={{ fontSize: 11 }} tickLine={false} axisLine={false} />
          <Tooltip contentStyle={{ borderRadius: '8px', border: '1px solid #e5e7eb', fontSize: '12px' }} />
          <Legend wrapperStyle={{ fontSize: '12px' }} />
          {chartType === 'area' ? (
            <Area type="monotone" dataKey={m.key} name={m.label} stroke={m.color} strokeWidth={2} fill={`url(#colorMetric)`} />
          ) : (
            <Bar dataKey={m.key} name={m.label} fill={m.color} radius={[4, 4, 0, 0]} />
          )}
        </Chart>
      </ResponsiveContainer>
    </div>
  )
}
