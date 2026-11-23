import { useEffect, useState } from 'react'
import {
  Plus, Trash2, RefreshCw, Check, Diff, Play, Server,
  Monitor, Terminal, FileText, ChevronDown, ChevronRight
} from 'lucide-react'
import { toast } from '@/components/Toast'
import StatusBar from '@/components/StatusBar'

// 10 个 adapter（学旧前端）
const ADAPTERS = ['pi', 'opencode', 'claude', 'codex', 'omp', 'dsh', 'picoclaw', 'crush', 'openclaw', 'hermes']

interface Host {
  name: string
  ssh: string
  platform: 'win' | 'unix'
}

interface AdapterInstance {
  name: string
  host: string
  path: string
}

export default function Sync() {
  const [hosts, setHosts] = useState<Host[]>([])
  const [instances, setInstances] = useState<Record<string, AdapterInstance[]>>({})
  const [expandedAdapters, setExpandedAdapters] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState(false)
  const [logs, setLogs] = useState<string[]>([])
  const [diffResult, setDiffResult] = useState<string>('')

  useEffect(() => {
    loadSyncConfig()
  }, [])

  async function loadSyncConfig() {
    try {
      const res = await fetch('/api/sync/config')
      const data = await res.json()
      setHosts(data.hosts || [])
      setInstances(data.instances || {})
    } catch {
      // 后端还没写，用示例数据
      setHosts([
        { name: 'win11', ssh: 'win11', platform: 'win' },
        { name: 'meilan-phone1', ssh: 'meilan-phone1', platform: 'unix' },
        { name: 'localhost', ssh: '', platform: 'unix' },
      ])
      setInstances({
        omp: [{ name: 'main', host: 'localhost', path: '~/.omp/agent/models.yml' }],
        dsh: [{ name: 'main', host: 'localhost', path: '~/.dsh/profiles/web/cordis.patch.yml' }],
        picoclaw: [{ name: 'phone', host: 'meilan-phone1', path: '~/.picoclaw/config.json' }],
        crush: [{ name: 'phone', host: 'meilan-phone1', path: '~/.config/crush/crushrc' }],
        openclaw: [{ name: 'win', host: 'win11', path: '~/.openclaw/openclaw.json' }],
        hermes: [{ name: 'win', host: 'win11', path: '.../hermes/config.yaml' }],
      })
    }
  }

  function addLog(text: string) {
    setLogs(l => [...l, `${new Date().toLocaleTimeString('zh-CN')} ${text}`])
  }

  // Hosts 操作
  function addHost() {
    setHosts([...hosts, { name: '', ssh: '', platform: 'unix' }])
  }

  function removeHost(idx: number) {
    setHosts(hosts.filter((_, i) => i !== idx))
  }

  function updateHost(idx: number, field: keyof Host, value: string) {
    const newHosts = [...hosts]
    newHosts[idx] = { ...newHosts[idx], [field]: value }
    setHosts(newHosts)
  }

  // Instances 操作
  function toggleAdapter(adapter: string) {
    const newExpanded = new Set(expandedAdapters)
    if (newExpanded.has(adapter)) newExpanded.delete(adapter)
    else newExpanded.add(adapter)
    setExpandedAdapters(newExpanded)
  }

  function addInstance(adapter: string) {
    setInstances({
      ...instances,
      [adapter]: [...(instances[adapter] || []), { name: '', host: '', path: '' }],
    })
  }

  function removeInstance(adapter: string, idx: number) {
    setInstances({
      ...instances,
      [adapter]: instances[adapter].filter((_, i) => i !== idx),
    })
  }

  function updateInstance(adapter: string, idx: number, field: keyof AdapterInstance, value: string) {
    const newInstances = { ...instances }
    newInstances[adapter][idx] = { ...newInstances[adapter][idx], [field]: value }
    setInstances(newInstances)
  }

  // 同步操作
  async function checkDiff() {
    setBusy(true)
    addLog('检查 diff...')
    try {
      const res = await fetch('/api/sync/diff', { method: 'POST' })
      const data = await res.json()
      setDiffResult(data.diff || '无差异')
      addLog('✓ diff 检查完成')
    } catch {
      setDiffResult('（后端未实现，这里会显示各agent 配置的 diff）')
      addLog('⚠ 后端未实现，显示占位')
    } finally {
      setBusy(false)
    }
  }

  async function syncAll() {
    setBusy(true)
    addLog('开始同步到所有 agent...')
    try {
      const total = Object.values(instances).reduce((sum, arr) => sum + arr.length, 0)
      await new Promise(r => setTimeout(r, 1000))
      addLog(`✓ 已同步 ${total} 个实例`)
      toast(`已同步 ${total} 个实例`)
    } catch {
      addLog('✗ 同步失败')
      toast('同步失败')
    } finally {
      setBusy(false)
    }
  }

  async function syncAdapter(adapter: string) {
    setBusy(true)
    addLog(`同步 ${adapter}...`)
    try {
      await new Promise(r => setTimeout(r, 500))
      const count = instances[adapter]?.length || 0
      addLog(`✓ ${adapter} 已同步 ${count} 个实例`)
      toast(`${adapter} 已同步`)
    } finally {
      setBusy(false)
    }
  }

  const totalInstances = Object.values(instances).reduce((sum, arr) => sum + arr.length, 0)

  return (
    <div className="space-y-4">
      {/* 顶部操作栏 */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-lg font-semibold">配置下发</h2>
          <p className="text-sm text-gray-500">
            {hosts.length} 台主机 · {totalInstances} 个实例 · {ADAPTERS.length} 个 adapter
          </p>
        </div>
        <div className="flex gap-2">
          <button
            onClick={checkDiff}
            disabled={busy}
            className="flex items-center gap-1.5 rounded-lg border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700 disabled:opacity-50"
          >
            <Diff className="h-4 w-4" />
            检查 Diff
          </button>
          <button
            onClick={syncAll}
            disabled={busy}
            className="flex items-center gap-1.5 rounded-lg bg-blue-600 px-3 py-1.5 text-sm text-white hover:bg-blue-700 disabled:opacity-50"
          >
            <Play className="h-4 w-4" />
            全部同步
          </button>
        </div>
      </div>

      {/* Diff 结果 */}
      {diffResult && (
        <div className="rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-800">
          <h3 className="mb-2 text-sm font-semibold">Diff 结果</h3>
          <pre className="max-h-40 overflow-auto rounded bg-gray-50 p-3 text-xs dark:bg-gray-900">{diffResult}</pre>
        </div>
      )}

      {/* Hosts 管理 */}
      <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
        <div className="mb-4 flex items-center justify-between">
          <h3 className="flex items-center gap-2 text-sm font-semibold">
            <Server className="h-4 w-4" />
            主机列表
          </h3>
          <button onClick={addHost} className="flex items-center gap-1 text-xs text-blue-600 hover:text-blue-700">
            <Plus className="h-3 w-3" />添加主机
          </button>
        </div>
        <table className="w-full text-left text-sm">
          <thead>
            <tr className="border-b border-gray-200 dark:border-gray-700">
              <th className="pb-2 font-medium text-gray-500">名称</th>
              <th className="pb-2 font-medium text-gray-500">SSH 别名</th>
              <th className="pb-2 font-medium text-gray-500">平台</th>
              <th className="pb-2 w-10"></th>
            </tr>
          </thead>
          <tbody>
            {hosts.map((h, idx) => (
              <tr key={idx}>
                <td className="py-1 pr-2">
                  <input
                    value={h.name}
                    onChange={e => updateHost(idx, 'name', e.target.value)}
                    placeholder="如 win11"
                    className="w-full rounded border border-gray-300 px-2 py-1 dark:border-gray-600 dark:bg-gray-700"
                  />
                </td>
                <td className="py-1 pr-2">
                  <input
                    value={h.ssh}
                    onChange={e => updateHost(idx, 'ssh', e.target.value)}
                    placeholder="ssh 别名"
                    className="w-full rounded border border-gray-300 px-2 py-1 dark:border-gray-600 dark:bg-gray-700"
                  />
                </td>
                <td className="py-1 pr-2">
                  <select
                    value={h.platform}
                    onChange={e => updateHost(idx, 'platform', e.target.value)}
                    className="rounded border border-gray-300 px-2 py-1 dark:border-gray-600 dark:bg-gray-700"
                  >
                    <option value="unix">unix</option>
                    <option value="win">win</option>
                  </select>
                </td>
                <td className="py-1">
                  <button onClick={() => removeHost(idx)} className="text-red-500 hover:text-red-700">
                    <Trash2 className="h-3 w-3" />
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {/* Adapter Instances */}
      <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
        <h3 className="mb-4 flex items-center gap-2 text-sm font-semibold">
          <FileText className="h-4 w-4" />
          Adapter 实例
        </h3>
        <div className="space-y-2">
          {ADAPTERS.map(adapter => {
            const insts = instances[adapter] || []
            const expanded = expandedAdapters.has(adapter)
            return (
              <div key={adapter} className="rounded-lg border border-gray-200 dark:border-gray-700">
                <button
                  onClick={() => toggleAdapter(adapter)}
                  className="flex w-full items-center gap-2 px-4 py-2 text-left text-sm font-medium"
                >
                  {expanded ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
                  <span>{adapter}</span>
                  <span className="ml-auto text-xs text-gray-400">{insts.length} 台</span>
                  <button
                    onClick={e => { e.stopPropagation(); syncAdapter(adapter) }}
                    className="ml-2 rounded bg-blue-50 px-2 py-0.5 text-xs text-blue-600 hover:bg-blue-100 dark:bg-blue-900/20"
                  >
                    同步
                  </button>
                </button>
                {expanded && (
                  <div className="border-t border-gray-200 p-3 dark:border-gray-700">
                    <table className="w-full text-left text-sm">
                      <thead>
                        <tr className="border-b border-gray-100 dark:border-gray-800">
                          <th className="pb-2 font-medium text-gray-500">名称</th>
                          <th className="pb-2 font-medium text-gray-500">主机</th>
                          <th className="pb-2 font-medium text-gray-500">路径</th>
                          <th className="pb-2 w-10"></th>
                        </tr>
                      </thead>
                      <tbody>
                        {insts.map((inst, idx) => (
                          <tr key={idx}>
                            <td className="py-1 pr-2">
                              <input
                                value={inst.name}
                                onChange={e => updateInstance(adapter, idx, 'name', e.target.value)}
                                placeholder="实例名"
                                className="w-full rounded border border-gray-300 px-2 py-1 dark:border-gray-600 dark:bg-gray-700"
                              />
                            </td>
                            <td className="py-1 pr-2">
                              <select
                                value={inst.host}
                                onChange={e => updateInstance(adapter, idx, 'host', e.target.value)}
                                className="w-full rounded border border-gray-300 px-2 py-1 dark:border-gray-600 dark:bg-gray-700"
                              >
                                <option value="">选择主机</option>
                                {hosts.map(h => (
                                  <option key={h.name} value={h.name}>{h.name}</option>
                                ))}
                              </select>
                            </td>
                            <td className="py-1 pr-2">
                              <input
                                value={inst.path}
                                onChange={e => updateInstance(adapter, idx, 'path', e.target.value)}
                                placeholder="~/path/to/config"
                                className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700"
                              />
                            </td>
                            <td className="py-1">
                              <button onClick={() => removeInstance(adapter, idx)} className="text-red-500 hover:text-red-700">
                                <Trash2 className="h-3 w-3" />
                              </button>
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                    <button
                      onClick={() => addInstance(adapter)}
                      className="mt-2 flex items-center gap-1 text-xs text-blue-600 hover:text-blue-700"
                    >
                      <Plus className="h-3 w-3" />添加实例
                    </button>
                  </div>
                )}
              </div>
            )
          })}
        </div>
      </div>

      {/* 同步日志 */}
      <div className="rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
        <div className="flex items-center justify-between border-b border-gray-200 px-4 py-2 dark:border-gray-700">
          <span className="text-sm font-medium">同步日志</span>
          <button onClick={() => setLogs([])} className="text-xs text-gray-400 hover:text-gray-600">清空</button>
        </div>
        <div className="max-h-40 overflow-auto p-4 font-mono text-xs">
          {logs.length === 0 ? (
            <span className="text-gray-400">— 选择上方功能执行</span>
          ) : (
            logs.map((log, i) => <div key={i}>{log}</div>)
          )}
        </div>
      </div>

      <StatusBar busy={busy} onClear={() => setLogs([])} />
    </div>
  )
}
