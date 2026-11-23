import { Dot } from 'lucide-react'

interface Props {
  busy: boolean
  text?: string
  onClear?: () => void
}

export default function StatusBar({ busy, text, onClear }: Props) {
  return (
    <div className="flex items-center gap-2 border-t border-gray-200 px-4 py-2 text-xs text-gray-500 dark:border-gray-700">
      <Dot className={`h-4 w-4 ${busy ? 'animate-pulse text-yellow-500' : 'text-green-500'}`} />
      <span>{text || (busy ? '执行中…' : '空闲')}</span>
      <div className="flex-1" />
      {onClear && (
        <button
          onClick={onClear}
          className="text-gray-400 hover:text-gray-600 dark:hover:text-gray-300"
        >
          清空日志
        </button>
      )}
    </div>
  )
}
