import { UiError, errorText, localizePlugin } from '../i18n/core'
import { useI18n } from '../i18n/useI18n'
import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react'
import { api } from '../api'
import type { Allocation, DeviceTelemetry, NpuTopology, PluginSummary, WorkerSnapshot } from '../types'

const emptyTopology: NpuTopology = {
  device: '/dev/dri/renderD129',
  platform: 'rk3576',
  driver: 'RKNPU',
  driver_version: null,
  runtime_version: null,
  total_tops_int8: 6,
  core_count: 2,
  current_frequency_hz: null,
  available_frequencies_hz: [],
  load_percent: null,
  governor: null,
  cores: [
    { id: 0, label: 'Core 0', allocation_id: null, plugin_id: null },
    { id: 1, label: 'Core 1', allocation_id: null, plugin_id: null },
  ],
}

const emptyTelemetry: DeviceTelemetry = {
  soc_temperature_c: null,
  npu_temperature_c: null,
  memory_total_bytes: null,
  memory_available_bytes: null,
  npu_core_load_percent: [null, null],
}

interface SystemContextValue {
  topology: NpuTopology
  telemetry: DeviceTelemetry
  allocations: Allocation[]
  plugins: PluginSummary[]
  workers: WorkerSnapshot[]
  connected: boolean
  loading: boolean
  refreshing: boolean
  actionId: string | null
  error: string | null
  updatedAt: Date | null
  refresh: (quiet?: boolean) => Promise<void>
  runAction: <T>(id: string, action: () => Promise<T>) => Promise<T | undefined>
  clearError: () => void
}

const SystemContext = createContext<SystemContextValue | null>(null)

export function SystemProvider({ children }: { children: React.ReactNode }) {
  const [topology, setTopology] = useState(emptyTopology)
  const [telemetry, setTelemetry] = useState(emptyTelemetry)
  const [allocations, setAllocations] = useState<Allocation[]>([])
  const [plugins, setPlugins] = useState<PluginSummary[]>([])
  const [workers, setWorkers] = useState<WorkerSnapshot[]>([])
  const [connected, setConnected] = useState(false)
  const [loading, setLoading] = useState(true)
  const [refreshing, setRefreshing] = useState(false)
  const [actionId, setActionId] = useState<string | null>(null)
  const [error, setError] = useState<unknown>(null)
  const [updatedAt, setUpdatedAt] = useState<Date | null>(null)

  const refreshCore = useCallback(async (quiet = false) => {
    if (!quiet) setRefreshing(true)
    try {
      const [nextTopology, nextAllocations, nextPlugins, nextWorkers] = await Promise.all([
        api.topology(), api.allocations(), api.plugins(), api.workers(),
      ])
      setTopology(nextTopology)
      setAllocations(nextAllocations)
      setPlugins(nextPlugins)
      setWorkers(nextWorkers)
      setConnected(true)
      setError(null)
      setUpdatedAt(new Date())
    } catch (cause) {
      setConnected(false)
      setError(cause instanceof Error ? cause : new UiError('error.connectCore'))
    } finally {
      setLoading(false)
      setRefreshing(false)
    }
  }, [])

  const refreshTelemetry = useCallback(async () => {
    try {
      setTelemetry(await api.telemetry().catch(() => emptyTelemetry))
    } catch {
      // telemetry is best-effort; failures are silently ignored
    }
  }, [])

  const refresh = useCallback(async (quiet = false) => {
    await refreshCore(quiet)
    await refreshTelemetry()
  }, [refreshCore, refreshTelemetry])

  useEffect(() => {
    void refresh()

    // Poll core data every 2 s, but pause when the tab is hidden to avoid
    // wasting CPU and network on an inactive browser tab.
    let coreTimer = 0
    let telemetryTimer = 0

    function startPolling() {
      coreTimer = window.setInterval(() => {
        if (document.visibilityState !== 'hidden') void refreshCore(true)
      }, 2_000)
      // Telemetry changes slowly (temperature, memory). Poll every 8 s.
      telemetryTimer = window.setInterval(() => {
        if (document.visibilityState !== 'hidden') void refreshTelemetry()
      }, 8_000)
    }

    function handleVisibility() {
      if (document.visibilityState === 'visible') void refreshCore(true)
    }

    startPolling()
    document.addEventListener('visibilitychange', handleVisibility)
    return () => {
      window.clearInterval(coreTimer)
      window.clearInterval(telemetryTimer)
      document.removeEventListener('visibilitychange', handleVisibility)
    }
  }, [refresh, refreshCore, refreshTelemetry])

  const runAction = useCallback(async <T,>(id: string, action: () => Promise<T>) => {
    setActionId(id)
    setError(null)
    try {
      const result = await action()
      await refresh(true)
      return result
    } catch (cause) {
      setError(cause instanceof Error ? cause : new UiError('error.operationFailed'))
      return undefined
    } finally {
      setActionId(null)
    }
  }, [refresh])

  const locale = useI18n()
  const localizedPlugins = useMemo(() => plugins.map((plugin) => localizePlugin(plugin, locale)), [plugins, locale])

  const value = useMemo<SystemContextValue>(() => ({
    topology, telemetry, allocations, plugins: localizedPlugins, workers, connected, loading, refreshing, actionId, error: error ? errorText(error) : null,
    updatedAt, refresh, runAction,
    clearError: () => setError(null),
  }), [topology, telemetry, allocations, localizedPlugins, workers, connected, loading, refreshing, actionId, error, updatedAt, refresh, runAction, locale])

  return <SystemContext.Provider value={value}>{children}</SystemContext.Provider>
}

export function useSystem() {
  const value = useContext(SystemContext)
  if (!value) throw new Error('useSystem must be used inside SystemProvider')
  return value
}
