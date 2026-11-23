import { useState } from 'react'
import Dashboard from './pages/Dashboard'
import Config from './pages/Config'
import Providers from './pages/Providers'
import Models from './pages/Models'
import Logs from './pages/Logs'
import Sync from './pages/Sync'
import Deploy from './pages/Deploy'
import SettingsPage from './pages/Settings'
import Sidebar from './components/Sidebar'
import Toast from './components/Toast'

export default function App() {
  const [tab, setTab] = useState('dashboard')

  return (
    <div className="flex h-screen overflow-hidden">
      <Sidebar active={tab} onSelect={setTab} />
      <main className="flex-1 overflow-y-auto p-6">
        <Toast />
        {tab === 'dashboard' && <Dashboard />}
        {tab === 'models' && <Models />}
        {tab === 'providers' && <Providers />}
        {tab === 'logs' && <Logs />}
        {tab === 'sync' && <Sync />}
        {tab === 'deploy' && <Deploy />}
        {tab === 'config' && <Config />}
      </main>
    </div>
  )
}
