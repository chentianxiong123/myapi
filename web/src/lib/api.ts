import type { StatsSummary, ProviderStat, RequestLog, TrendPoint, DailyPoint } from './types'

const BASE = ''

export async function fetchJSON<T>(path: string): Promise<T> {
  const res = await fetch(`${BASE}${path}`)
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`)
  return res.json()
}

export async function postJSON<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  })
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`)
  return res.json()
}

export const api = {
  getStats: () => fetchJSON<StatsSummary>('/stats/summary'),
  getProviderStats: () => fetchJSON<ProviderStat[]>('/stats/providers'),
  getRequestLogs: (limit = 100) => fetchJSON<RequestLog[]>(`/api/logs?limit=${limit}`),
  getTrend: (hours = 24) => fetchJSON<TrendPoint[]>(`/stats/trend?hours=${hours}`),
  getDaily: (days = 371) => fetchJSON<DailyPoint[]>(`/stats/daily?days=${days}`),
}
