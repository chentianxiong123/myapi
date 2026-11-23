import { useEffect, useState } from 'react'
import {
  Plus, Key, Copy, Trash2, Pencil, RefreshCw, Check, Ban, Power, Clock, Activity,
} from 'lucide-react'
import { toast } from '@/components/Toast'
import StatusBar from '@/components/StatusBar'
import type { ApiToken } from '@/lib/types'

const DEFAULT_TOKENS: ApiToken[] = [
  { id: 't-1', name: 'pi', key: 'sk-myapi-pi-abc123', status: 'enabled', unlimited_quota: true, remain_quota: 0, expired_time: '', models: [], ip_limit: '', created_at: '2026-11-01 10:00:00', last_used_at: '2026-11-23 12:30:00', total_requests: 45231 },
  { id: 't-2', name: 'opencode', key: 'sk-myapi-opencode-def456', status: 'enabled', unlimited_quota: true, remain_quota: 0, expired_time: '', models: [], ip_limit: '', created_at: '2026-11-02 09:00:00', last_used_at: '2026-11-23 11:20:00', total_requests: 18742 },
  { id: 't-3', name: 'codex', key: 'sk-myapi-codex-ghi789', status: 'enabled', unlimited_quota: true, remain_quota: 0, expired_time: '', models: [], ip_limit: '', created_at: '2026-11-03 14:00:00', last_used_at: '2026-11-22 18:45:00', total_requests: 9312 },
  { id: 't-4', name: 'claude', key: 'sk-myapi-claude-jkl012', status: 'disabled', unlimited_quota: true, remain_quota: 0, expired_time: '2026-12-01', models: ['claude-*'], ip_limit: '192.168.31.*', created_at: '2026-11-05 08:00:00', last_used_at: '', total_requests: 0 },
]

const MODELS = ['deepseek-v4-flash', 'sensenova-6.8-flash-lite', 'gpt-4o', 'claude-3-5-sonnet', 'gemini-2.5-pro']

function genKey() {
  const rand = Math.random().toString(36).slice(2, 10)
  return `sk-myapi-${rand}`
}

export default function Tokens() {
  const [tokens, setTokens] = useState<ApiToken[]>([])
  const [busy, setBusy] = useState(false)
  const [loading, setLoading] = useState(true)
  const [editing, setEditing] = useState<ApiToken | null>(null)
  const [showKey, setShowKey] = useState<Set<string>>(new Set())

  useEffect(() => { load() }, [])

  async function load() {
    try {
      const res = await fetch('/api/tokens')
      const data = await res.json()
      setTokens(data.tokens || [])
    } catch {
      setTokens(DEFAULT_TOKENS)
    } finally { setLoading(false) }
  }

  function toggleShow(id: string) {
    const s = new Set(showKey); s.has(id) ? s.delete(id) : s.add(id); setShowKey(s)
  }

  async function copyKey(t: ApiToken) {
    try { await navigator.clipboard.writeText(t.key); toast('密钥已复制') }
    catch { toast('复制失败') }
  }

  function newToken() {
    const t: ApiToken = {
      id: `t-${Date.now()}`, name: '', key: genKey(), status: 'enabled',
      unlimited_quota: true, remain_quota: 0, expired_time: '', models: [], ip_limit: '',
      created_at: new Date().toISOString().slice(0, 19).replace('T', ' '), last_used_at: '', total_requests: 0,
    }
    setEditing(t)
  }

  function saveToken() {
    if (!editing) return
    if (!editing.name.trim()) { toast('请填名称'); return }
    const exists = tokens.some(t => t.id === editing.id)
    setTokens(exists ? tokens.map(t => t.id === editing.id ? editing : t) : [...tokens, editing])
    setEditing(null)
    toast(exists ? '密钥已更新' : '密钥已创建')
  }

  function toggleStatus(t: ApiToken) {
    setTokens(tokens.map(x => x.id === t.id ? { ...x, status: x.status === 'enabled' ? 'disabled' : 'enabled' } : x))
  }

  function removeToken(t: ApiToken) {
    if (!confirm(`删除密钥「${t.name}」？`)) return
    setTokens(tokens.filter(x => x.id !== t.id))
    toast('已删除')
  }

  const enabled = tokens.filter(t => t.status === 'enabled').length
  const totalReq = tokens.reduce((s, t) => s + t.total_requests, 0)

  return (
    <div className="space-y-4">
      {/* 头部 */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-lg font-semibold">密钥管理</h2>
          <p className="text-sm text-gray-500">给各 agent（pi/opencode/codex/claude）发 API 密钥</p>
        </div>
        <div className="flex gap-2">
          <button onClick={newToken} className="flex items-center gap-1.5 rounded-lg bg-blue-600 px-3 py-1.5 text-sm text-white hover:bg-blue-700">
            <Plus className="h-4 w-4" />新建密钥
          </button>
        </div>
      </div>

      {/* 概览 */}
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        {[
          { label: '密钥数', value: `${tokens.length}`, sub: `${enabled} 启用`, icon: Key, color: 'text-blue-600' },
          { label: '启用中', value: `${enabled}`, sub: '可调用', icon: Power, color: 'text-green-600' },
          { label: '禁用', value: `${tokens.length - enabled}`, sub: '停用', icon: Ban, color: 'text-red-500' },
          { label: '累计请求', value: totalReq.toLocaleString(), sub: '所有密钥', icon: Activity, color: 'text-orange-600' },
        ].map(card => (
          <div key={card.label} className="rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-800">
            <div className="flex items-center gap-2 text-xs text-gray-400"><card.icon className={`h-4 w-4 ${card.color}`} />{card.label}</div>
            <div className="mt-1 text-2xl font-semibold tabular-nums">{card.value}</div>
            <div className="text-xs text-gray-400">{card.sub}</div>
          </div>
        ))}
      </div>

      {/* 编辑弹窗 */}
      {editing && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4" onClick={() => setEditing(null)}>
          <div className="w-full max-w-lg rounded-xl bg-white p-6 shadow-xl dark:bg-gray-800" onClick={e => e.stopPropagation()}>
            <div className="mb-4 flex items-center justify-between">
              <h3 className="flex items-center gap-2 text-sm font-semibold"><Key className="h-4 w-4" />{tokens.some(t => t.id === editing.id) ? '编辑密钥' : '新建密钥'}</h3>
              <button onClick={() => setEditing(null)} className="text-gray-400 hover:text-gray-600">✕</button>
            </div>
            <div className="space-y-3">
              <div><label className="mb-1 block text-xs text-gray-500">名称（用途）</label>
                <input value={editing.name} onChange={e => setEditing({ ...editing, name: e.target.value })} placeholder="如 pi / opencode / codex"
                  className="w-full rounded border border-gray-300 px-2 py-1.5 text-sm dark:border-gray-600 dark:bg-gray-700" /></div>
              <div><label className="mb-1 block text-xs text-gray-500">密钥</label>
                <div className="flex gap-2">
                  <input value={editing.key} onChange={e => setEditing({ ...editing, key: e.target.value })} readOnly
                    className="w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" />
                  <button onClick={() => setEditing({ ...editing, key: genKey() })} className="shrink-0 rounded border border-gray-300 px-2 text-xs hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700">换</button>
                </div></div>
              <div className="grid grid-cols-2 gap-3">
                <div><label className="mb-1 block text-xs text-gray-500">过期时间</label>
                  <input value={editing.expired_time} onChange={e => setEditing({ ...editing, expired_time: e.target.value })} placeholder="2026-12-01 或留空永不过期"
                    className="w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" /></div>
                <div><label className="mb-1 block text-xs text-gray-500">IP 限制</label>
                  <input value={editing.ip_limit} onChange={e => setEditing({ ...editing, ip_limit: e.target.value })} placeholder="192.168.31.* 或留空不限"
                    className="w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" /></div>
              </div>
              <div><label className="mb-1 flex items-center gap-1.5 text-xs text-gray-500">
                <input type="checkbox" checked={editing.unlimited_quota} onChange={e => setEditing({ ...editing, unlimited_quota: e.target.checked })} />
                不限额度（取消则设剩余额度）
              </label>
                {!editing.unlimited_quota && (
                  <input type="number" value={editing.remain_quota} onChange={e => setEditing({ ...editing, remain_quota: parseInt(e.target.value) || 0 })}
                    className="mt-1 w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" placeholder="剩余额度（tokens）" />
                )}</div>
              <div><label className="mb-1 block text-xs text-gray-500">允许的模型（逗号分隔，空=全部）</label>
                <input value={editing.models.join(', ')} onChange={e => setEditing({ ...editing, models: e.target.value.split(',').map(s => s.trim()).filter(Boolean) })} placeholder="留空 = 全部模型"
                  className="w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" /></div>
            </div>
            <div className="mt-5 flex justify-end gap-2">
              <button onClick={() => setEditing(null)} className="rounded-lg border border-gray-300 px-4 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700">取消</button>
              <button onClick={saveToken} className="rounded-lg bg-blue-600 px-4 py-1.5 text-sm text-white hover:bg-blue-700">保存</button>
            </div>
          </div>
        </div>
      )}

      {/* 密钥列表 */}
      <div className="rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
        <div className="flex items-center justify-between border-b border-gray-200 px-4 py-3 dark:border-gray-700">
          <h3 className="text-sm font-semibold">密钥列表</h3>
          <span className="text-xs text-gray-400">{tokens.length} 个</span>
        </div>
        {loading ? (
          <div className="h-32 animate-pulse bg-gray-200 dark:bg-gray-700" />
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-left text-sm">
              <thead><tr className="border-b border-gray-200 dark:border-gray-700">
                <th className="px-3 py-2 font-medium text-gray-500">名称</th>
                <th className="px-3 py-2 font-medium text-gray-500">密钥</th>
                <th className="px-3 py-2 font-medium text-gray-500">状态</th>
                <th className="px-3 py-2 font-medium text-gray-500">模型</th>
                <th className="px-3 py-2 font-medium text-gray-500">过期</th>
                <th className="px-3 py-2 font-medium text-gray-500">请求</th>
                <th className="px-3 py-2 font-medium text-gray-500">最后使用</th>
                <th className="px-3 py-2 w-20"></th>
              </tr></thead>
              <tbody>
                {tokens.map(t => (
                  <tr key={t.id} className="border-b border-gray-100 hover:bg-gray-50 dark:border-gray-800 dark:hover:bg-gray-700/50">
                    <td className="px-3 py-2 font-medium">{t.name || '—'}</td>
                    <td className="px-3 py-2">
                      <div className="flex items-center gap-1 font-mono text-xs">
                        <span>{showKey.has(t.id) ? t.key : t.key.slice(0, 12) + '••••••••'}</span>
                        <button onClick={() => toggleShow(t.id)} className="text-gray-400 hover:text-gray-600"><Check className="h-3 w-3" /></button>
                        <button onClick={() => copyKey(t)} className="text-gray-400 hover:text-gray-600"><Copy className="h-3 w-3" /></button>
                      </div>
                    </td>
                    <td className="px-3 py-2">
                      <span className={`rounded px-1.5 py-0.5 text-xs ${t.status === 'enabled' ? 'bg-green-100 text-green-700 dark:bg-green-900/30' : 'bg-gray-100 text-gray-500 dark:bg-gray-700'}`}>
                        {t.status === 'enabled' ? '启用' : '禁用'}
                      </span>
                    </td>
                    <td className="px-3 py-2 text-xs text-gray-400">{t.models.length ? t.models.join(', ') : '全部'}</td>
                    <td className="px-3 py-2 text-xs text-gray-400">{t.expired_time || <span className="text-green-600">永不过期</span>}</td>
                    <td className="px-3 py-2 font-mono text-xs">{t.total_requests.toLocaleString()}</td>
                    <td className="px-3 py-2 text-xs text-gray-400">{t.last_used_at || '—'}</td>
                    <td className="px-3 py-2">
                      <div className="flex gap-1">
                        <button onClick={() => setEditing({ ...t })} title="编辑" className="text-gray-400 hover:text-blue-600"><Pencil className="h-3.5 w-3.5" /></button>
                        <button onClick={() => toggleStatus(t)} title={t.status === 'enabled' ? '禁用' : '启用'} className={`${t.status === 'enabled' ? 'text-red-400 hover:text-red-600' : 'text-green-500 hover:text-green-700'}`}><Power className="h-3.5 w-3.5" /></button>
                        <button onClick={() => removeToken(t)} title="删除" className="text-gray-400 hover:text-red-600"><Trash2 className="h-3.5 w-3.5" /></button>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      <StatusBar busy={busy} onClear={() => {}} />
    </div>
  )
}
