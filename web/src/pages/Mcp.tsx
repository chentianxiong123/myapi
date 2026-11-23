import { useState } from 'react'
import {
  Plug, Search, Eye, Plus, Trash2, Save, RefreshCw, Power, Server,
} from 'lucide-react'
import { toast } from '@/components/Toast'
import type { McpToolConfig, McpServerConfig } from '@/lib/types'

// MCP 接口：搜索/识图作为 MCP 工具暴露给 agent（学 9router encodeMcpTools）
const DEFAULT_TOOLS: McpToolConfig[] = [
  { tool: 'search', enabled: true, models: ['tavily/search', 'exa/search'], max_results: 5, search_type: 'web' },
  { tool: 'vision', enabled: true, models: ['gemini-3.5-flash-lite', 'sensenova-6.8-flash'], max_results: 0, search_type: '' },
]

const DEFAULT_SERVERS: McpServerConfig[] = [
  { id: 'mcp-1', name: 'myapi-search', command: 'myapi', args: '--mcp search', env: { MYAPI_KEY: 'sk-myapi-master' }, transport: 'stdio', url: '', enabled: true },
  { id: 'mcp-2', name: 'myapi-vision', command: 'myapi', args: '--mcp vision', env: {}, transport: 'stdio', url: '', enabled: false },
]

export default function Mcp() {
  const [tools, setTools] = useState<McpToolConfig[]>(DEFAULT_TOOLS)
  const [servers, setServers] = useState<McpServerConfig[]>(DEFAULT_SERVERS)
  const [busy, setBusy] = useState(false)

  function toggleTool(i: number) {
    setTools(tools.map((t, idx) => idx === i ? { ...t, enabled: !t.enabled } : t))
  }
  function updateTool(i: number, field: keyof McpToolConfig, value: any) {
    setTools(tools.map((t, idx) => idx === i ? { ...t, [field]: value } : t))
  }
  function toggleServer(i: number) {
    setServers(servers.map((s, idx) => idx === i ? { ...s, enabled: !s.enabled } : s))
  }
  function updateServer(i: number, field: keyof McpServerConfig, value: any) {
    setServers(servers.map((s, idx) => idx === i ? { ...s, [field]: value } : s))
  }
  function addServer() {
    setServers([...servers, { id: `mcp-${Date.now()}`, name: '', command: 'myapi', args: '', env: {}, transport: 'stdio', url: '', enabled: true }])
  }
  function removeServer(i: number) {
    setServers(servers.filter((_, idx) => idx !== i))
  }

  async function saveAll() {
    setBusy(true)
    try {
      await fetch('/api/mcp', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ tools, servers }) })
      toast('MCP 配置已保存')
    } catch {
      toast('后端未实现，仅本地保存')
    } finally { setBusy(false) }
  }

  return (
    <div className="space-y-4">
      {/* 头部 */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-lg font-semibold">MCP 接口</h2>
          <p className="text-sm text-gray-500">搜索/识图作为 MCP 工具暴露给 agent（学 9router mcp_tools）</p>
        </div>
        <button onClick={saveAll} disabled={busy} className="flex items-center gap-1.5 rounded-lg bg-blue-600 px-3 py-1.5 text-sm text-white hover:bg-blue-700 disabled:opacity-50">
          <Save className="h-4 w-4" />保存
        </button>
      </div>

      {/* 概览 */}
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        {[
          { label: 'MCP 工具', value: `${tools.filter(t => t.enabled).length}/${tools.length}`, sub: '启用', icon: Plug, color: 'text-blue-600' },
          { label: '搜索', value: tools.find(t => t.tool === 'search')?.enabled ? '开' : '关', sub: 'webSearch', icon: Search, color: 'text-teal-600' },
          { label: '识图', value: tools.find(t => t.tool === 'vision')?.enabled ? '开' : '关', sub: 'vision', icon: Eye, color: 'text-violet-600' },
          { label: 'MCP Server', value: `${servers.filter(s => s.enabled).length}`, sub: '启动', icon: Server, color: 'text-orange-600' },
        ].map(card => (
          <div key={card.label} className="rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-800">
            <div className="flex items-center gap-2 text-xs text-gray-400"><card.icon className={`h-4 w-4 ${card.color}`} />{card.label}</div>
            <div className="mt-1 text-2xl font-semibold tabular-nums">{card.value}</div>
            <div className="text-xs text-gray-400">{card.sub}</div>
          </div>
        ))}
      </div>

      {/* 工具映射（模型 → MCP 工具） */}
      <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
        <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold"><Plug className="h-4 w-4" />工具映射（模型 → MCP 工具）</h3>
        <div className="space-y-3">
          {tools.map((t, i) => (
            <div key={t.tool} className="rounded-lg border border-gray-200 p-3 dark:border-gray-700">
              <div className="flex items-center gap-2">
                {t.tool === 'search' ? <Search className="h-4 w-4 text-teal-600" /> : <Eye className="h-4 w-4 text-violet-600" />}
                <span className="font-medium">{t.tool === 'search' ? '搜索（webSearch）' : '识图（vision）'}</span>
                <label className="ml-auto flex items-center gap-1 text-xs text-gray-500">
                  <input type="checkbox" checked={t.enabled} onChange={() => toggleTool(i)} />启用
                </label>
              </div>
              <div className="mt-2 grid grid-cols-1 gap-2 sm:grid-cols-3">
                <div className="sm:col-span-3">
                  <label className="mb-1 block text-xs text-gray-500">绑定模型（逗号分隔）</label>
                  <input value={t.models.join(', ')} onChange={e => updateTool(i, 'models', e.target.value.split(',').map(s => s.trim()).filter(Boolean))}
                    placeholder={t.tool === 'search' ? 'tavily/search, exa/search' : 'gemini-3.5-flash-lite, sensenova-6.8-flash'}
                    className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" />
                </div>
                {t.tool === 'search' && (
                  <>
                    <div><label className="mb-1 block text-xs text-gray-500">最大结果数</label>
                      <input type="number" value={t.max_results} onChange={e => updateTool(i, 'max_results', parseInt(e.target.value) || 5)}
                        className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" /></div>
                    <div><label className="mb-1 block text-xs text-gray-500">搜索类型</label>
                      <select value={t.search_type} onChange={e => updateTool(i, 'search_type', e.target.value)}
                        className="w-full rounded border border-gray-300 px-2 py-1 text-xs dark:border-gray-600 dark:bg-gray-700">
                        <option value="web">web</option>
                        <option value="news">news</option>
                        <option value="x">x（Xquik）</option>
                      </select></div>
                  </>
                )}
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* MCP Server 配置（下发到 agent 的 mcpServers） */}
      <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
        <div className="mb-3 flex items-center justify-between">
          <h3 className="flex items-center gap-2 text-sm font-semibold"><Server className="h-4 w-4" />MCP Server（agent 的 mcpServers 配置）</h3>
          <button onClick={addServer} className="flex items-center gap-1 text-xs text-blue-600 hover:text-blue-700"><Plus className="h-3 w-3" />添加 Server</button>
        </div>
        <div className="space-y-2">
          {servers.map((s, i) => (
            <div key={s.id} className="rounded-lg border border-gray-200 p-3 dark:border-gray-700">
              <div className="flex items-center gap-2">
                <input value={s.name} onChange={e => updateServer(i, 'name', e.target.value)} placeholder="server 名（如 myapi-search）"
                  className="w-40 rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" />
                <select value={s.transport} onChange={e => updateServer(i, 'transport', e.target.value)}
                  className="rounded border border-gray-300 px-2 py-1 text-xs dark:border-gray-600 dark:bg-gray-700">
                  <option value="stdio">stdio</option>
                  <option value="http">http</option>
                  <option value="sse">sse</option>
                </select>
                <label className="flex items-center gap-1 text-xs text-gray-500">
                  <input type="checkbox" checked={s.enabled} onChange={() => toggleServer(i)} />启用
                </label>
                <div className="flex-1" />
                <button onClick={() => removeServer(i)} className="text-red-500 hover:text-red-700"><Trash2 className="h-3 w-3" /></button>
              </div>
              <div className="mt-2 grid grid-cols-1 gap-2 sm:grid-cols-2">
                <div><label className="mb-1 block text-xs text-gray-500">命令</label>
                  <input value={s.command} onChange={e => updateServer(i, 'command', e.target.value)} placeholder="myapi"
                    className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" /></div>
                <div><label className="mb-1 block text-xs text-gray-500">参数</label>
                  <input value={s.args} onChange={e => updateServer(i, 'args', e.target.value)} placeholder="--mcp search"
                    className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" /></div>
                {s.transport !== 'stdio' && (
                  <div className="sm:col-span-2"><label className="mb-1 block text-xs text-gray-500">端点 URL</label>
                    <input value={s.url} onChange={e => updateServer(i, 'url', e.target.value)} placeholder="http://myapi:3000/mcp"
                      className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" /></div>
                )}
              </div>
            </div>
          ))}
        </div>
        <p className="mt-2 text-xs text-gray-400">agent（claude/opencode/codex）的 mcpServers 引用这些，搜索/识图走 MCP 而非 HTTP 端点</p>
      </div>
    </div>
  )
}
