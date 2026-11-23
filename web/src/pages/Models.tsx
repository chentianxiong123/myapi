import { useEffect, useState } from 'react'
import { Layers } from 'lucide-react'
import type { ExposedModel } from '@/lib/types'

export default function Models() {
  const [models, setModels] = useState<ExposedModel[]>([])
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    loadModels()
  }, [])

  async function loadModels() {
    try {
      const res = await fetch('/v1/models')
      const data = await res.json()
      // 后端返回的是 OpenAI 格式，这里简化处理
      setModels(data.data || [])
    } catch {
      // 后端还没写，用示例数据（从 cpa 配置提取）
      setModels([
        {
          alias: 'deepseek-v4-flash',
          providers: [
            { provider_name: 'SenseNova', real_name: 'deepseek-v4-flash', weight: 1 },
            { provider_name: 'SenseNova', real_name: 'deepseek-flash', weight: 1 },
            { provider_name: 'SenseNova', real_name: 'deepseek-v4.1-flash', weight: 1 },
          ],
        },
        {
          alias: 'agnes-3.0-flash',
          providers: [
            { provider_name: 'agnes', real_name: 'agnes-3.0-flash', weight: 1 },
            { provider_name: 'pie-xian', real_name: 'agnes-2.5-flash', weight: 1 },
          ],
        },
        {
          alias: 'gemini-3.5-flash-lite',
          providers: [
            { provider_name: 'pie-xian', real_name: 'gemini-3.1-flash-lite', weight: 1 },
            { provider_name: 'pie-xian', real_name: 'gemini-3.5-flash-lite', weight: 1 },
          ],
        },
        {
          alias: 'hy4',
          providers: [
            { provider_name: 'pie-xian', real_name: 'hy4-flash-free', weight: 1 },
          ],
        },
        {
          alias: 'sensenova-6.8-flash',
          providers: [
            { provider_name: 'SenseNova', real_name: 'sensenova-6.8-flash', weight: 1 },
            { provider_name: 'iamhc', real_name: 'sensenova-6.8-flash', weight: 1 },
          ],
        },
      ])
    } finally {
      setLoading(false)
    }
  }

  if (loading) return <div className="animate-pulse text-gray-500">加载中…</div>

  return (
    <div className="space-y-4">
      <h2 className="text-lg font-semibold">Exposed Models ({models.length})</h2>
      <p className="text-sm text-gray-500">
        按 alias 分组，同一 alias 可以有多个 provider（多目标路由）
      </p>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        {models.map((m, idx) => (
          <div key={idx} className="rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-800">
            <div className="mb-3 flex items-center gap-2">
              <Layers className="h-4 w-4 text-blue-600" />
              <span className="font-mono font-semibold">{m.alias}</span>
              <span className="ml-auto rounded bg-blue-50 px-2 py-0.5 text-xs text-blue-600 dark:bg-blue-900/20">
                {m.providers.length} provider{m.providers.length > 1 ? 's' : ''}
              </span>
            </div>
            <div className="space-y-1">
              {m.providers.map((p, pi) => (
                <div key={pi} className="flex items-center gap-2 text-sm">
                  <span className="w-24 text-gray-500">{p.provider_name}</span>
                  <span className="flex-1 font-mono text-xs text-gray-400">{p.real_name}</span>
                </div>
              ))}
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}
