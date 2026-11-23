import { useEffect, useState } from 'react'

let showToastFn: ((msg: string) => void) | null = null

export function toast(msg: string) {
  showToastFn?.(msg)
}

export default function Toast() {
  const [msg, setMsg] = useState('')
  const [visible, setVisible] = useState(false)

  useEffect(() => {
    showToastFn = (m: string) => {
      setMsg(m)
      setVisible(true)
      setTimeout(() => setVisible(false), 2200)
    }
    return () => { showToastFn = null }
  }, [])

  return (
    <div
      className={`fixed bottom-6 left-1/2 -translate-x-1/2 rounded-lg bg-gray-900 px-4 py-2 text-sm text-white shadow-lg transition-opacity dark:bg-gray-700 ${
        visible ? 'opacity-100' : 'opacity-0'
      } pointer-events-none`}
    >
      {msg}
    </div>
  )
}
