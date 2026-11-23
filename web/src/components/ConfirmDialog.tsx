import { AlertTriangle, Info } from 'lucide-react'

interface Props {
  isOpen: boolean
  title: string
  message: string
  confirmText?: string
  cancelText?: string
  variant?: 'destructive' | 'info'
  onConfirm: () => void
  onCancel: () => void
}

export default function ConfirmDialog({
  isOpen, title, message, confirmText = '确认', cancelText = '取消', variant = 'destructive', onConfirm, onCancel
}: Props) {
  if (!isOpen) return null
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={onCancel}>
      <div className="w-full max-w-md rounded-xl bg-white p-6 shadow-xl dark:bg-gray-800" onClick={e => e.stopPropagation()}>
        <div className="mb-4 flex items-center gap-3">
          <div className={`rounded-lg p-2 ${variant === 'destructive' ? 'bg-red-50 text-red-600 dark:bg-red-900/20' : 'bg-blue-50 text-blue-600 dark:bg-blue-900/20'}`}>
            {variant === 'destructive' ? <AlertTriangle className="h-5 w-5" /> : <Info className="h-5 w-5" />}
          </div>
          <h3 className="text-lg font-semibold">{title}</h3>
        </div>
        <p className="mb-6 text-sm text-gray-600 dark:text-gray-300">{message}</p>
        <div className="flex justify-end gap-2">
          <button onClick={onCancel} className="rounded-lg border border-gray-300 px-4 py-2 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700">
            {cancelText}
          </button>
          <button onClick={onConfirm} className={`rounded-lg px-4 py-2 text-sm text-white ${variant === 'destructive' ? 'bg-red-600 hover:bg-red-700' : 'bg-blue-600 hover:bg-blue-700'}`}>
            {confirmText}
          </button>
        </div>
      </div>
    </div>
  )
}
