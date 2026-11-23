import { type ReactNode } from 'react'
import { X } from 'lucide-react'
import { cn } from '@/lib/utils'

interface DialogProps {
  open: boolean
  onClose: () => void
  title?: string
  children: ReactNode
}

export function Dialog({ open, onClose, title, children }: DialogProps) {
  if (!open) return null
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={onClose}>
      <div className={cn('w-full max-w-lg rounded-xl bg-white p-6 shadow-xl dark:bg-gray-800')} onClick={e => e.stopPropagation()}>
        {title && (
          <div className="mb-4 flex items-center justify-between">
            <h2 className="text-lg font-semibold">{title}</h2>
            <button onClick={onClose} className="rounded p-1 hover:bg-gray-100 dark:hover:bg-gray-700"><X className="h-4 w-4" /></button>
          </div>
        )}
        {children}
      </div>
    </div>
  )
}
