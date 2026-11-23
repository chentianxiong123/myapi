import React from 'react'
import { RefreshCw, AlertTriangle } from 'lucide-react'

interface State { hasError: boolean; error: Error | null }

export class ErrorBoundary extends React.Component<{ children: React.ReactNode }, State> {
  state: State = { hasError: false, error: null }

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error }
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error('ErrorBoundary caught:', error, info.componentStack)
  }

  render() {
    if (this.state.hasError) {
      return (
        <div className="flex min-h-screen items-center justify-center bg-gray-50 dark:bg-gray-900">
          <div className="text-center">
            <div className="mx-auto mb-4 w-12 text-red-500">
              <AlertTriangle className="h-12 w-12" />
            </div>
            <h1 className="mb-2 text-xl font-bold">页面出错了</h1>
            <p className="mb-4 text-gray-500">{this.state.error?.message}</p>
            <button
              onClick={() => window.location.reload()}
              className="inline-flex items-center gap-2 rounded-lg bg-blue-600 px-4 py-2 text-white hover:bg-blue-700"
            >
              <RefreshCw className="h-4 w-4" />刷新页面
            </button>
          </div>
        </div>
      )
    }
    return this.props.children
  }
}
