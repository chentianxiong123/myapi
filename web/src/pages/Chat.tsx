import { useEffect, useRef, useState } from 'react'
import { Send, Loader2, Eraser, RotateCcw, Terminal } from 'lucide-react'
import { toast } from '@/components/Toast'

interface ChatMsg {
  role: 'user' | 'assistant'
  content: string
}

export default function Chat() {
  const [models, setModels] = useState<string[]>([])
  const [model, setModel] = useState('')
  const [messages, setMessages] = useState<ChatMsg[]>([])
  const [input, setInput] = useState('')
  const [systemPrompt, setSystemPrompt] = useState('')
  const [busy, setBusy] = useState(false)
  const [elapsed, setElapsed] = useState(0)
  const bottomRef = useRef<HTMLDivElement>(null)
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null)

  useEffect(() => {
    fetch('/v1/models').then(r => r.json()).then(d => {
      const list = (d.data || []).map((m: any) => m.id || m.alias || m).filter(Boolean)
      setModels(list)
      if (list.length && !model) setModel(list[0])
    }).catch(() => {
      setModels(['deepseek-v4-flash', 'agnes-3.0-flash', 'gemini-3.5-flash-lite', 'hy4', 'sensenova-6.8-flash'])
      setModel('deepseek-v4-flash')
    })
  }, [])

  useEffect(() => { bottomRef.current?.scrollIntoView({ behavior: 'smooth' }) }, [messages])

  async function send() {
    const text = input.trim()
    if (!text || busy) return
    if (!model) { toast('请选模型'); return }
    const msgs: ChatMsg[] = [...messages, { role: 'user', content: text }]
    setMessages(msgs)
    setInput('')
    setBusy(true)
    setElapsed(0)
    timerRef.current = setInterval(() => setElapsed(e => e + 1), 1000)
    try {
      // 走网关 /v1/chat/completions 测链路（模型 + key + 路由）
      const res = await fetch('/v1/chat/completions', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          model,
          messages: [
            ...(systemPrompt.trim() ? [{ role: 'system', content: systemPrompt }] : []),
            ...msgs.slice(-8).map(m => ({ role: m.role, content: m.content })),
          ],
          stream: false,
        }),
      })
      const data = await res.json().catch(() => ({}))
      const content = data.choices?.[0]?.message?.content
        || data.choices?.[0]?.text
        || (typeof data === 'string' ? data : JSON.stringify(data))
      setMessages([...msgs, { role: 'assistant', content: content || `(空响应 ${res.status})` }])
      if (!res.ok) toast(`请求失败：${res.status}`)
    } catch (e: any) {
      setMessages([...msgs, { role: 'assistant', content: `⚠ 错误：${e?.message || e}` }])
    } finally {
      setBusy(false)
      if (timerRef.current) clearInterval(timerRef.current)
    }
  }

  function clearChat() { setMessages([]); setElapsed(0) }

  return (
    <div className="space-y-4">
      {/* 头部 */}
      <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
        <div>
          <h2 className="text-lg font-semibold">在线测试</h2>
          <p className="text-sm text-gray-500">选模型发消息，验证网关链路（模型 + 密钥 + 路由）</p>
        </div>
      </div>

      {/* 参数栏 */}
      <div className="rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-800">
        <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
          <div>
            <label className="mb-1 block text-xs text-gray-500">模型</label>
            <select value={model} onChange={e => setModel(e.target.value)} className="w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-sm dark:border-gray-600 dark:bg-gray-700">
              {models.length === 0 && <option value="">（无模型）</option>}
              {models.map(m => <option key={m} value={m}>{m}</option>)}
            </select>
          </div>
          <div>
            <label className="mb-1 block text-xs text-gray-500">System Prompt（可选）</label>
            <input value={systemPrompt} onChange={e => setSystemPrompt(e.target.value)} placeholder="你是一个有用的助手"
              className="w-full rounded border border-gray-300 px-2 py-1.5 text-sm dark:border-gray-600 dark:bg-gray-700" />
          </div>
        </div>
      </div>

      {/* 对话区 */}
      <div className="flex h-[52vh] flex-col rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
        <div className="flex items-center justify-between border-b border-gray-200 px-4 py-2 dark:border-gray-700">
          <span className="flex items-center gap-1.5 text-sm font-medium"><Terminal className="h-4 w-4" />对话</span>
          {busy && <span className="text-xs text-gray-400">耗时 {elapsed}s…</span>}
          <button onClick={clearChat} className="flex items-center gap-1 text-xs text-gray-400 hover:text-gray-600"><Eraser className="h-3 w-3" />清空</button>
        </div>
        <div className="flex-1 space-y-3 overflow-y-auto p-4">
          {messages.length === 0 && <div className="flex h-full items-center justify-center text-gray-400">发送一条消息测试模型连通性</div>}
          {messages.map((m, i) => (
            <div key={i} className={`flex ${m.role === 'user' ? 'justify-end' : 'justify-start'}`}>
              <div className={`max-w-[80%] whitespace-pre-wrap rounded-lg px-3 py-2 text-sm ${m.role === 'user' ? 'bg-blue-600 text-white' : 'bg-gray-100 text-gray-800 dark:bg-gray-700 dark:text-gray-200'}`}>
                {m.content}
              </div>
            </div>
          ))}
          <div ref={bottomRef} />
        </div>
        <div className="border-t border-gray-200 p-3 dark:border-gray-700">
          <div className="flex gap-2">
            <textarea
              value={input}
              onChange={e => setInput(e.target.value)}
              onKeyDown={e => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send() } }}
              placeholder="输入消息，Enter 发送，Shift+Enter 换行"
              rows={2}
              className="flex-1 resize-none rounded-lg border border-gray-300 px-3 py-2 text-sm dark:border-gray-600 dark:bg-gray-700"
            />
            <button onClick={send} disabled={busy || !input.trim()} className="flex w-20 items-center justify-center rounded-lg bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50">
              {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Send className="h-4 w-4" />}
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}
