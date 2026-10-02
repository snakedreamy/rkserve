import { BrowserRouter, Navigate, Route, Routes } from 'react-router-dom'
import { ApiPage } from '../pages/ApiPage'
import { EventsPage } from '../pages/EventsPage'
import { NpuPage } from '../pages/NpuPage'
import { PluginDetailPage } from '../pages/PluginDetailPage'
import { PluginRunPage } from '../pages/PluginRunPage'
import { PluginsPage } from '../pages/PluginsPage'
import { AppLayout } from './AppLayout'
import { AuthProvider } from './AuthContext'
import { SystemProvider } from './SystemContext'

export default function App() {
  return (
    <BrowserRouter>
      <AuthProvider>
        <SystemProvider>
          <Routes>
            <Route element={<AppLayout />}>
              <Route index element={<Navigate to="/npu" replace />} />
              <Route path="npu" element={<NpuPage />} />
              <Route path="plugins" element={<PluginsPage />} />
              <Route path="plugins/:pluginId" element={<PluginDetailPage />} />
              <Route path="plugins/:pluginId/run" element={<PluginRunPage />} />
              <Route path="apis" element={<ApiPage />} />
              <Route path="events" element={<EventsPage />} />
              <Route path="*" element={<Navigate to="/npu" replace />} />
            </Route>
          </Routes>
        </SystemProvider>
      </AuthProvider>
    </BrowserRouter>
  )
}
