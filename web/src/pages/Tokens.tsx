import { useState } from 'react'
import {
  Plus, Key, Copy, Trash2, Pencil, Ban, Power, Activity, User, Check, RefreshCw,
} from 'lucide-react'
import { toast } from '@/components/Toast'
import StatusBar from '@/components/StatusBar'
import type { ApiToken } from '@/lib/types'

const DEFAULT_MASTER: ApiToken = {
  id: 'master', name: '我的主密钥', key: 'sk-myapi-master-abc123', status: 'enabled',
  unlimited_quota: true, remain_quota: 0, expired_time: '', models: [], ip_limit: '',
  created_at: '2026-11-01 10:00:00', last_used_at: '2026-11-23 12:30:00', total_requests: 73301,
}

const DEFAULT_FRIENDS: ApiToken[] = [
  { id: 'f-1', name: '老张', key: 'sk-myapi-friend-xxx111', status: 'enabled', unlimited_quota: false, remain_quota: 1000000, expired_time: '2026-12-31', models: ['deepseek-*'], ip_limit: '', created_at: '2026-11-10 09:00:00', last_used_at: '2026-11-22 20:15:00', total_requests: 1203 },
  { id: 'f-2', name: '小李', key: 'sk-myapi-friend-yyy222', status: 'disabled', unlimited_quota: false, remain_quota: 500000, expired_time: '', models: [], ip_limit: '192.168.31.*', created_at: '2026-11-15 14:00:00', last_used_at: '', total_requests: 0 },
]

function genKey() {
  return `sk-myapi-${Math.random().toString(36).slice(2, 10)}`
}

export default function Tokens() {
  const [master, setMaster] = useState<ApiToken>(DEFAULT_MASTER)
  const [friends, setFriends] = useState<ApiToken[]>(DEFAULT_FRIENDS)
  const [busy, setBusy] = useState(false)
  const [editing, setEditing] = useState<ApiToken | null>(null)
  const [showKey, setShowKey] = useState(false)
  const [showFriendKeys, setShowFriendKeys] = useState<Set<string>>(new Set())

  async function copyKey(key: string) {
    try { await navigator.clipboard.writeText(key); toast('密钥已复制') }
    catch { toast('复制失败') }
  }

  function toggleMaster() {
    setMaster({ ...master, status: master.status === 'enabled' ? 'disabled' : 'enabled' })
    toast(master.status === 'enabled' ? '主密钥已停用' : '主密钥已启用')
  }

  function newFriend() {
    const t: ApiToken = {
      id: `f-${Date.now()}`, name: '', key: genKey(), status: 'enabled',
      unlimited_quota: false, remain_quota: 1000000, expired_time: '', models: [], ip_limit: '',
      created_at: new Date().toISOString().slice(0, 19).replace('T', ' '), last_used_at: '', total_requests: 0,
    }
    setEditing(t)
  }

  function saveFriend() {
    if (!editing) return
    if (!editing.name.trim()) { toast('请填好友名'); return }
    const exists = friends.some(t => t.id === editing.id)
    setFriends(exists ? friends.map(t => t.id === editing.id ? editing : t) : [...friends, editing])
    setEditing(null)
    toast(exists ? '好友密钥已更新' : '好友密钥已创建')
  }

  function toggleStatus(t: ApiToken) {
    setFriends(friends.map(x => x.id === t.id ? { ...x, status: x.status === 'enabled' ? 'disabled' : 'enabled' } : x))
  }

  function removeFriend(t: ApiToken) {
    if (!confirm(`删除好友「${t.name}」的密钥？`)) return
    setFriends(friends.filter(x => x.id !== t.id))
    toast('已删除')
  }

  const friendEnabled = friends.filter(f => f.status === 'enabled').length
  const friendReq = friends.reduce((s, f) => s + f.total_requests, 0)

  return (
    <div className="space-y-4">
      {/* 头部 */}
      <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
        <div>
          <h2 className="text-lg font-semibold">密钥管理</h2>
          <p className="text-sm text-gray-500">自己的 agent 工具共用主密钥；分享给好友单独发密钥</p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button onClick={newFriend} className="flex items-center gap-1.5 rounded-lg bg-blue-600 px-3 py-1.5 text-sm text-white hover:bg-blue-700">
            <Plus className="h-4 w-4" />分享给好友
          </button>
        </div>
      </div>

      {/* 我的主密钥（独立卡片，非表格） */}
      <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
        <div className="mb-4 flex items-center justify-between">
          <h3 className="flex items-center gap-2 text-sm font-semibold"><Key className="h-4 w-4 text-blue-500" />我的主密钥</h3>
          <div className="flex items-center gap-2">
            <span className={`rounded px-2 py-0.5 text-xs ${master.status === 'enabled' ? 'bg-green-100 text-green-700 dark:bg-green-900/30' : 'bg-gray-100 text-gray-500 dark:bg-gray-700'}`}>
              {master.status === 'enabled' ? '启用' : '停用'}
            </span>
            <button onClick={toggleMaster} className={`flex items-center gap-1 rounded border px-2 py-1 text-xs ${master.status === 'enabled' ? 'border-red-200 text-red-500 hover:bg-red-50' : 'border-green-200 text-green-600 hover:bg-green-50'}`}>
              <Power className="h-3 w-3" />{master.status === 'enabled' ? '停用' : '启用'}
            </button>
          </div>
        </div>
        <div className="flex items-center gap-2 rounded-lg bg-gray-50 p-3 dark:bg-gray-700/50">
          <span className="font-mono text-sm">{showKey ? master.key : master.key.slice(0, 14) + '••••••••••'}</span>
          <button onClick={() => setShowKey(!showKey)} className="text-gray-400 hover:text-gray-600" title="显示/隐藏"><Check className="h-4 w-4" /></button>
          <button onClick={() => copyKey(master.key)} className="text-gray-400 hover:text-blue-600" title="复制"><Copy className="h-4 w-4" /></button>
          <div className="flex-1" />
          <div className="text-xs text-gray-400">所有 agent 工具共用 · 累计 {master.total_requests.toLocaleString()} 请求 · 最后 {master.last_used_at || '—'}</div>
        </div>
        <p className="mt-2 text-xs text-gray-400">自己的 pi / opencode / codex / claude 等全部工具共用这把主密钥；工具区分靠 User-Agent。</p>
      </div>

      {/* 好友密钥（表格） */}
      <div className="rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
        <div className="flex items-center justify-between border-b border-gray-200 px-4 py-3 dark:border-gray-700">
          <h3 className="flex items-center gap-2 text-sm font-semibold"><User className="h-4 w-4 text-violet-500" />好友密钥</h3>
          <span className="text-xs text-gray-400">{friends.length} 好友 · {friendEnabled} 启用 · {friendReq.toLocaleString()} 请求</span>
        </div>
        <div className="overflow-x-auto">
          <table className="w-full text-left text-sm">
            <thead><tr className="border-b border-gray-200 dark:border-gray-700">
              <th className="px-3 py-2 font-medium text-gray-500">好友</th>
              <th className="px-3 py-2 font-medium text-gray-500">密钥</th>
              <th className="px-3 py-2 font-medium text-gray-500">状态</th>
              <th className="px-3 py-2 font-medium text-gray-500">额度</th>
              <th className="px-3 py-2 font-medium text-gray-500">模型</th>
              <th className="px-3 py-2 font-medium text-gray-500">过期</th>
              <th className="px-3 py-2 font-medium text-gray-500">请求</th>
              <th className="px-3 py-2 w-24"></th>
            </tr></thead>
            <tbody>
              {friends.map(f => (
                <tr key={f.id} className="border-b border-gray-100 hover:bg-gray-50 dark:border-gray-800 dark:hover:bg-gray-700/50">
                  <td className="px-3 py-2 font-medium">{f.name}</td>
                  <td className="px-3 py-2">
                    <div className="flex items-center gap-1 font-mono text-xs">
                      <span>{showFriendKeys.has(f.id) ? f.key : f.key.slice(0, 14) + '••••••'}</span>
                      <button onClick={() => { const s = new Set(showFriendKeys); s.has(f.id) ? s.delete(f.id) : s.add(f.id); setShowFriendKeys(s) }} className="text-gray-400 hover:text-gray-600"><Check className="h-3 w-3" /></button>
                      <button onClick={() => copyKey(f.key)} className="text-gray-400 hover:text-blue-600"><Copy className="h-3 w-3" /></button>
                    </div>
                  </td>
                  <td className="px-3 py-2">
                    <span className={`rounded px-1.5 py-0.5 text-xs ${f.status === 'enabled' ? 'bg-green-100 text-green-700 dark:bg-green-900/30' : 'bg-gray-100 text-gray-500 dark:bg-gray-700'}`}>
                      {f.status === 'enabled' ? '启用' : '禁用'}
                    </span>
                  </td>
                  <td className="px-3 py-2 text-xs text-gray-500">
                    {f.unlimited_quota ? <span className="text-green-600">不限</span> : `${(f.remain_quota / 10000).toFixed(0)}万`}
                  </td>
                  <td className="px-3 py-2 text-xs text-gray-400">{f.models.length ? f.models.join(', ') : '全部'}</td>
                  <td className="px-3 py-2 text-xs text-gray-400">{f.expired_time || <span className="text-green-600">永不过期</span>}</td>
                  <td className="px-3 py-2 font-mono text-xs">{f.total_requests.toLocaleString()}</td>
                  <td className="px-3 py-2">
                    <div className="flex gap-1">
                      <button onClick={() => setEditing({ ...f })} title="编辑" className="text-gray-400 hover:text-blue-600"><Pencil className="h-3.5 w-3.5" /></button>
                      <button onClick={() => toggleStatus(f)} title={f.status === 'enabled' ? '禁用' : '启用'} className={f.status === 'enabled' ? 'text-red-400 hover:text-red-600' : 'text-green-500 hover:text-green-700'}><Power className="h-3.5 w-3.5" /></button>
                      <button onClick={() => removeFriend(f)} title="删除" className="text-gray-400 hover:text-red-600"><Trash2 className="h-3.5 w-3.5" /></button>
                    </div>
                  </td>
                </tr>
              ))}
              {friends.length === 0 && (
                <tr><td colSpan={8} className="px-4 py-8 text-center text-gray-400">还没有好友密钥——点右上「分享给好友」创建</td></tr>
              )}
            </tbody>
          </table>
        </div>
      </div>

      {/* 编辑好友密钥弹窗 */}
      {editing && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4" onClick={() => setEditing(null)}>
          <div className="w-full max-w-lg rounded-xl bg-white p-6 shadow-xl dark:bg-gray-800" onClick={e => e.stopPropagation()}>
            <div className="mb-4 flex items-center justify-between">
              <h3 className="flex items-center gap-2 text-sm font-semibold"><User className="h-4 w-4" />{friends.some(t => t.id === editing.id) ? '编辑好友密钥' : '分享给好友'}</h3>
              <button onClick={() => setEditing(null)} className="text-gray-400 hover:text-gray-600">✕</button>
            </div>
            <div className="space-y-3">
              <div><label className="mb-1 block text-xs text-gray-500">好友名</label>
                <input value={editing.name} onChange={e => setEditing({ ...editing, name: e.target.value })} placeholder="如 老张"
                  className="w-full rounded border border-gray-300 px-2 py-1.5 text-sm dark:border-gray-600 dark:bg-gray-700" /></div>
              <div><label className="mb-1 block text-xs text-gray-500">密钥</label>
                <div className="flex gap-2">
                  <input value={editing.key} readOnly className="w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" />
                  <button onClick={() => setEditing({ ...editing, key: genKey() })} className="shrink-0 rounded border border-gray-300 px-2 text-xs hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700">换</button>
                </div></div>
              <div className="grid grid-cols-2 gap-3">
                <div><label className="mb-1 block text-xs text-gray-500">额度（万 tokens）</label>
                  <input type="number" value={editing.unlimited_quota ? '' : editing.remain_quota / 10000}
                    onChange={e => setEditing({ ...editing, unlimited_quota: false, remain_quota: (parseInt(e.target.value) || 0) * 10000 })}
                    placeholder="如 100"
                    className="w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" /></div>
                <div><label className="mb-1 block text-xs text-gray-500">过期时间</label>
                  <input value={editing.expired_time} onChange={e => setEditing({ ...editing, expired_time: e.target.value })} placeholder="2026-12-31 或留空永不过期"
                    className="w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" /></div>
              </div>
              <div><label className="mb-1 flex items-center gap-1.5 text-xs text-gray-500">
                <input type="checkbox" checked={editing.unlimited_quota} onChange={e => setEditing({ ...editing, unlimited_quota: e.target.checked })} />
                不限额度
              </label></div>
              <div><label className="mb-1 block text-xs text-gray-500">允许的模型（逗号分隔，空=全部）</label>
                <input value={editing.models.join(', ')} onChange={e => setEditing({ ...editing, models: e.target.value.split(',').map(s => s.trim()).filter(Boolean) })} placeholder="如 deepseek-*,gpt-4o 或留空全部"
                  className="w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" /></div>
              <div><label className="mb-1 block text-xs text-gray-500">IP 限制（逗号分隔，空=不限）</label>
                <input value={editing.ip_limit} onChange={e => setEditing({ ...editing, ip_limit: e.target.value })} placeholder="192.168.31.*"
                  className="w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" /></div>
            </div>
            <div className="mt-5 flex justify-end gap-2">
              <button onClick={() => setEditing(null)} className="rounded-lg border border-gray-300 px-4 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700">取消</button>
              <button onClick={saveFriend} className="rounded-lg bg-blue-600 px-4 py-1.5 text-sm text-white hover:bg-blue-700">保存</button>
            </div>
          </div>
        </div>
      )}

      <StatusBar busy={busy} onClear={() => {}} />
    </div>
  )
}
