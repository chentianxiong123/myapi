import { useState } from 'react'
import {
  Plus, Trash2, Globe, Zap, RefreshCw, Pencil, Check, X, Power, Shield,
} from 'lucide-react'
import { toast } from '@/components/Toast'
import type { ProxyPool } from '@/lib/types'

// 学 9router proxyPools：池 = 代理 URL + 类型 + 状态 + 策略（Provider 引用池）
const DEFAULT_POOLS: ProxyPool[] = [
  { id: 'p-1', name: '家里 Clash', proxyUrl: 'http://127.0.0.1:7890', type: 'http', isActive: true, strictProxy: false, testStatus: 'ok' },
  { id: 'p-2', name: '服务器代理', proxyUrl: 'socks5://192.168.31.12:1080', type: 'socks5', isActive: true, strictProxy: true, testStatus: 'unknown' },
  { id: 'p-3', name: '备用（失效）', proxyUrl: 'http://192.168.31.14:8080', type: 'http', isActive: false, strictProxy: false, testStatus: 'fail' },
]

export default function ProxyPools() {
  const [pools, setPools] = useState<ProxyPool[]>(DEFAULT_POOLS)
  const [busy, setBusy] = useState(false)
  const [editing, setEditing] = useState<ProxyPool | null>(null)
  const [testing, setTesting] = useState<Set<string>>(new Set())

  function addPool() {
    setEditing({ id: `p-${Date.now()}`, name: '', proxyUrl: '', type: 'http', isActive: true, strictProxy: false, testStatus: 'unknown' })
  }
  function savePool() {
    if (!editing) return
    if (!editing.name.trim() || !editing.proxyUrl.trim()) { toast('请填池名和代理 URL'); return }
    const exists = pools.some(p => p.id === editing.id)
    setPools(exists ? pools.map(p => p.id === editing.id ? editing : p) : [...pools, editing])
    setEditing(null)
    toast(exists ? '代理池已更新' : '代理池已创建')
  }
  function removePool(p: ProxyPool) {
    if (!confirm(`删除代理池「${p.name}」？`)) return
    setPools(pools.filter(x => x.id !== p.id))
    toast('已删除')
  }
  function toggleActive(p: ProxyPool) {
    setPools(pools.map(x => x.id === p.id ? { ...x, isActive: !x.isActive } : x))
  }
  async function testPool(p: ProxyPool) {
    setTesting(s => new Set(s).add(p.id))
    try {
      // 模拟测试（学 9router testStatus）：用代理 URL 探测
      await new Promise(r => setTimeout(r, 600))
      setPools(pools.map(x => x.id === p.id ? { ...x, testStatus: 'ok' } : x))
      toast(`${p.name} 代理可用`)
    } catch {
      setPools(pools.map(x => x.id === p.id ? { ...x, testStatus: 'fail' } : x))
    } finally {
      setTesting(s => { const ns = new Set(s); ns.delete(p.id); return ns })
    }
  }

  const active = pools.filter(p => p.isActive).length

  return (
    <div className="space-y-4">
      {/* 头部 */}
      <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
        <div>
          <h2 className="text-lg font-semibold">代理池</h2>
          <p className="text-sm text-gray-500">学 9router proxyPools：多个代理池，Provider 引用 + 选择策略</p>
        </div>
        <div className="flex gap-2">
          <button onClick={addPool} className="flex items-center gap-1.5 rounded-lg bg-blue-600 px-3 py-1.5 text-sm text-white hover:bg-blue-700">
            <Plus className="h-4 w-4" />新建代理池
          </button>
        </div>
      </div>

      {/* 概览 */}
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
        {[
          { label: '代理池', value: `${pools.length}`, sub: `${active} 启用`, icon: Globe, color: 'text-blue-600' },
          { label: '可用', value: `${pools.filter(p => p.testStatus === 'ok').length}`, sub: '测试通过', icon: Check, color: 'text-green-600' },
          { label: '失效', value: `${pools.filter(p => p.testStatus === 'fail').length}`, sub: '测试失败', icon: X, color: 'text-red-500' },
        ].map(card => (
          <div key={card.label} className="rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-800">
            <div className="flex items-center gap-2 text-xs text-gray-400"><card.icon className={`h-4 w-4 ${card.color}`} />{card.label}</div>
            <div className="mt-1 text-2xl font-semibold tabular-nums">{card.value}</div>
            <div className="text-xs text-gray-400">{card.sub}</div>
          </div>
        ))}
      </div>

      {/* 池列表 */}
      <div className="rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
        <div className="flex items-center justify-between border-b border-gray-200 px-4 py-3 dark:border-gray-700">
          <h3 className="text-sm font-semibold">代理池列表</h3>
          <span className="text-xs text-gray-400">{pools.length} 池</span>
        </div>
        <div className="divide-y divide-gray-100 dark:divide-gray-800">
          {pools.map(p => (
            <div key={p.id} className="flex items-center gap-3 p-3">
              <div className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-lg ${p.testStatus === 'ok' ? 'bg-green-100 text-green-600 dark:bg-green-900/20' : p.testStatus === 'fail' ? 'bg-red-100 text-red-500 dark:bg-red-900/20' : 'bg-gray-100 text-gray-500 dark:bg-gray-700'}`}>
                <Globe className="h-4 w-4" />
              </div>
              <div className="flex-1">
                <div className="flex items-center gap-2">
                  <span className="font-medium">{p.name}</span>
                  <span className="rounded bg-gray-100 px-1.5 py-0.5 text-[10px] uppercase text-gray-500 dark:bg-gray-700">{p.type}</span>
                  {p.strictProxy && <span className="rounded bg-amber-100 px-1.5 py-0.5 text-[10px] text-amber-600 dark:bg-amber-900/20">严格</span>}
                  <span className={`rounded px-1.5 py-0.5 text-[10px] ${p.testStatus === 'ok' ? 'bg-green-100 text-green-600 dark:bg-green-900/20' : p.testStatus === 'fail' ? 'bg-red-100 text-red-500 dark:bg-red-900/20' : 'bg-gray-100 text-gray-500 dark:bg-gray-700'}`}>
                    {p.testStatus === 'ok' ? '可用' : p.testStatus === 'fail' ? '失效' : '未测'}
                  </span>
                </div>
                <div className="mt-0.5 font-mono text-xs text-gray-400">{p.proxyUrl}</div>
              </div>
              <button
                onClick={() => testPool(p)}
                disabled={testing.has(p.id)}
                className={`flex items-center gap-1 rounded border px-2 py-1 text-xs ${p.testStatus === 'ok' ? 'border-green-200 bg-green-50 text-green-700 dark:bg-green-900/20' : 'border-gray-300 hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700'}`}
              >
                {testing.has(p.id) ? <RefreshCw className="h-3 w-3 animate-spin" /> : <Zap className="h-3 w-3" />}
                测试
              </button>
              <button onClick={() => toggleActive(p)} title={p.isActive ? '停用' : '启用'}
                className={p.isActive ? 'text-green-500 hover:text-green-700' : 'text-gray-300 hover:text-gray-500'}>
                <Power className="h-4 w-4" />
              </button>
              <button onClick={() => setEditing({ ...p })} title="编辑" className="text-gray-400 hover:text-blue-600"><Pencil className="h-4 w-4" /></button>
              <button onClick={() => removePool(p)} title="删除" className="text-gray-400 hover:text-red-600"><Trash2 className="h-4 w-4" /></button>
            </div>
          ))}
          {pools.length === 0 && <div className="p-8 text-center text-gray-400">还没有代理池——点右上「新建代理池」</div>}
        </div>
      </div>

      {/* 编辑弹窗 */}
      {editing && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4" onClick={() => setEditing(null)}>
          <div className="w-full max-w-md rounded-xl bg-white p-6 shadow-xl dark:bg-gray-800" onClick={e => e.stopPropagation()}>
            <div className="mb-4 flex items-center justify-between">
              <h3 className="flex items-center gap-2 text-sm font-semibold"><Shield className="h-4 w-4" />{pools.some(p => p.id === editing.id) ? '编辑代理池' : '新建代理池'}</h3>
              <button onClick={() => setEditing(null)} className="text-gray-400 hover:text-gray-600">✕</button>
            </div>
            <div className="space-y-3">
              <div><label className="mb-1 block text-xs text-gray-500">池名</label>
                <input value={editing.name} onChange={e => setEditing({ ...editing, name: e.target.value })} placeholder="如 家里 Clash"
                  className="w-full rounded border border-gray-300 px-2 py-1.5 text-sm dark:border-gray-600 dark:bg-gray-700" /></div>
              <div><label className="mb-1 block text-xs text-gray-500">代理 URL</label>
                <input value={editing.proxyUrl} onChange={e => setEditing({ ...editing, proxyUrl: e.target.value })} placeholder="http://127.0.0.1:7890 或 socks5://..."
                  className="w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-sm dark:border-gray-600 dark:bg-gray-700" /></div>
              <div className="grid grid-cols-2 gap-3">
                <div><label className="mb-1 block text-xs text-gray-500">类型</label>
                  <select value={editing.type} onChange={e => setEditing({ ...editing, type: e.target.value as any })}
                    className="w-full rounded border border-gray-300 px-2 py-1.5 text-sm dark:border-gray-600 dark:bg-gray-700">
                    <option value="http">HTTP</option>
                    <option value="socks5">SOCKS5</option>
                  </select></div>
                <div><label className="mb-1 flex items-center gap-1.5 text-xs text-gray-500">
                  <input type="checkbox" checked={editing.strictProxy} onChange={e => setEditing({ ...editing, strictProxy: e.target.checked })} />
                  严格模式（池必须保证）
                </label></div>
              </div>
            </div>
            <div className="mt-5 flex justify-end gap-2">
              <button onClick={() => setEditing(null)} className="rounded-lg border border-gray-300 px-4 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700">取消</button>
              <button onClick={savePool} className="rounded-lg bg-blue-600 px-4 py-1.5 text-sm text-white hover:bg-blue-700">保存</button>
            </div>
          </div>
        </div>
      )}

      <div className="flex justify-end gap-2">
        <button onClick={() => setPools(DEFAULT_POOLS)} className="text-xs text-gray-400 hover:text-gray-600">重置示例</button>
      </div>
    </div>
  )
}
