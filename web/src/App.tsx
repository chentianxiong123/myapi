import { useState } from 'react'
import Dashboard from './pages/Dashboard'
import Config from './pages/Config'
import Providers from './pages/Providers'
import ProxyPools from './pages/ProxyPools'
import Mcp from './pages/Mcp'
import Models from './pages/Models'
import Logs from './pages/Logs'
import Chat from './pages/Chat'
import Sync from './pages/Sync'
import Tokens from './pages/Tokens'
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
        {tab === 'tokens' && <Tokens />}
        {tab === 'providers' && <Providers />}
        {tab === 'proxypools' && <ProxyPools />}
        {tab === 'mcp' && <Mcp />}
        {tab === 'logs' && <Logs />}
        {tab === 'chat' && <Chat />}
        {tab === 'sync' && <Sync />}
        {tab === 'deploy' && <Deploy />}
        {tab === 'config' && <Config />}
      </main>
    </div>
  )
}
