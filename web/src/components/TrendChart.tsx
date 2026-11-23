import { AreaChart, Area, XAxis, YAxis, Tooltip, ResponsiveContainer } from 'recharts'
import type { TrendPoint } from '@/lib/types'

interface Props {
  data: TrendPoint[]
  loading: boolean
}

export default function TrendChart({ data, loading }: Props) {
  if (loading) {
    return (
      <div className="h-64 animate-pulse rounded-xl bg-gray-200 dark:bg-gray-700" />
    )
  }

  return (
    <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
      <h3 className="mb-4 text-sm font-semibold text-gray-700 dark:text-gray-300">
        请求趋势（最近 24 小时）
      </h3>
      <ResponsiveContainer width="100%" height={256}>
        <AreaChart data={data}>
          <defs>
            <linearGradient id="colorRequests" x1="0" y1="0" x2="0" y2="1">
              <stop offset="5%" stopColor="#2563eb" stopOpacity={0.3} />
              <stop offset="95%" stopColor="#2563eb" stopOpacity={0} />
            </linearGradient>
          </defs>
          <XAxis
            dataKey="time"
            tick={{ fontSize: 11 }}
            tickLine={false}
            axisLine={false}
          />
          <YAxis tick={{ fontSize: 11 }} tickLine={false} axisLine={false} />
          <Tooltip
            contentStyle={{
              borderRadius: '8px',
              border: '1px solid #e5e7eb',
              fontSize: '12px',
            }}
          />
          <Area
            type="monotone"
            dataKey="requests"
            stroke="#2563eb"
            strokeWidth={2}
            fill="url(#colorRequests)"
          />
        </AreaChart>
      </ResponsiveContainer>
    </div>
  )
}
