import { t, number } from '../i18n/core'
import { useI18n } from '../i18n/useI18n'
import { Icon } from '../icons/Icon'
import { useState } from 'react'
import { Link } from 'react-router-dom'
import { api } from '../api'
import { PageHeading } from '../components/PageHeading'
import { useSystem } from '../app/SystemContext'
import type { CoreMask } from '../types'

const masks = () => [
  { value: 'core0', label: t('npu.coreZero') },
  { value: 'core1', label: t('npu.coreOne') },
  { value: 'core0_1', label: t('npu.dualCore') },
] satisfies Array<{ value: CoreMask; label: string }>

const padCount = { x: 10, y: 8 }

export function NpuPage() {
  useI18n()
  const { topology, telemetry, allocations, plugins, workers, actionId, runAction } = useSystem()
  const [schedulerView, setSchedulerView] = useState<'available' | 'leases'>('available')
  const occupiedCount = topology.cores.filter((core) => core.allocation_id).length
  const readyWorkerCount = workers.filter((worker) => worker.status === 'ready').length
  const candidates = plugins.filter(
    (plugin) => plugin.state !== 'ready' && !allocations.some((allocation) => allocation.plugin_id === plugin.id),
  )
  const allocate = (pluginId: string, mask: CoreMask) => runAction(pluginId, () => api.allocate(pluginId, mask))
  const release = (leaseId: string) => runAction(leaseId, () => api.release(leaseId))
  const thermal = telemetry.npu_temperature_c !== null && telemetry.npu_temperature_c >= 80

  return (
    <>
      <PageHeading
        eyebrow={t('common.npu')}
        title={t('nav.compute')}
        description={`${topology.platform.toUpperCase()} · ${topology.device}`}
        aside={
          <div className="heading-state">
            <span>{number(readyWorkerCount)}</span>
            <small>{t('npu.readyWorkers')}</small>
          </div>
        }
      />

      <div className="npu-workbench">
        <section className="npu-console" aria-labelledby="topology-title">
          <div className="npu-console-head">
            <div className="npu-device">
              <span className="eyebrow">{t('npu.topology')}</span>
              <h2 id="topology-title">{topology.platform.toUpperCase()}</h2>
              <code>{topology.device}</code>
            </div>
            <div className="npu-facts" aria-label={t('npu.summary')}>
              <div>
                <span>{t('npu.occupancy')}</span>
                <strong>
                  {number(occupiedCount)}/{number(topology.core_count)}
                </strong>
              </div>
              <div>
                <span>{t('npu.readyWorkers')}</span>
                <strong>{number(readyWorkerCount)}</strong>
              </div>
              <div>
                <span>{t('npu.specifications')}</span>
                <strong>{number(topology.total_tops_int8)} TOPS</strong>
              </div>
            </div>
          </div>

          <div className={`die-package ${thermal ? 'thermal' : ''}`} aria-label={t('npu.package')}>
            <div className="die-pads die-pads-y" aria-hidden="true">
              {Array.from({ length: padCount.y }, (_, index) => (
                <i key={`l${index}`} />
              ))}
            </div>
            <div className="die-stack">
              <div className="die-pads die-pads-x" aria-hidden="true">
                {Array.from({ length: padCount.x }, (_, index) => (
                  <i key={`t${index}`} />
                ))}
              </div>
              <div className="die-body">
                <span className="die-mark">
                  {topology.platform.toUpperCase()} · {number(topology.total_tops_int8)} TOPS INT8
                </span>
                <div
                  className="die-cores"
                  style={{ gridTemplateColumns: `repeat(${Math.max(topology.core_count, 1)}, minmax(0, 1fr))` }}
                >
                  {topology.cores.map((core) => {
                    const load = telemetry.npu_core_load_percent[core.id]
                    const allocated = core.allocation_id !== null
                    return (
                      <div className={`die-tile ${allocated ? 'hot' : 'idle'}`} key={core.id}>
                        <div
                          className="die-fill"
                          style={{ height: `${fillHeight(allocated, load)}%` }}
                          aria-hidden="true"
                        />
                        <div className="die-tile-head">
                          <strong>{t('npu.coreLabel', { id: number(core.id) })}</strong>
                          <span>{coreStatus(allocated, load)}</span>
                        </div>
                        {allocated ? (
                          <div className="die-occupant">
                            <span className="lease-pulse" />
                            <strong>{core.plugin_id}</strong>
                            <code>{core.allocation_id}</code>
                          </div>
                        ) : (
                          <div className="die-idle">{t('npu.idleHint')}</div>
                        )}
                      </div>
                    )
                  })}
                </div>
              </div>
              <div className="die-pads die-pads-x" aria-hidden="true">
                {Array.from({ length: padCount.x }, (_, index) => (
                  <i key={`b${index}`} />
                ))}
              </div>
            </div>
            <div className="die-pads die-pads-y" aria-hidden="true">
              {Array.from({ length: padCount.y }, (_, index) => (
                <i key={`r${index}`} />
              ))}
            </div>
          </div>

          <div className="telemetry-strip" aria-label={t('npu.telemetry')}>
            <div>
              <span>{t('npu.temperature')}</span>
              <strong className={thermal ? 'hot-reading' : ''}>{formatTemperature(telemetry.npu_temperature_c)}</strong>
              <small>npu-thermal</small>
            </div>
            <div>
              <span>{t('npu.socTemperature')}</span>
              <strong>{formatTemperature(telemetry.soc_temperature_c)}</strong>
              <small>soc-thermal</small>
            </div>
            <div>
              <span>{t('npu.memory')}</span>
              <strong>{formatMemory(telemetry.memory_total_bytes, telemetry.memory_available_bytes)}</strong>
              <small>{formatMemoryPercent(telemetry.memory_total_bytes, telemetry.memory_available_bytes)}</small>
            </div>
            <div>
              <span>{t('npu.frequency')}</span>
              <strong>{formatFrequency(topology.current_frequency_hz)}</strong>
              <small>{topology.governor ?? t('npu.unknownGovernor')}</small>
            </div>
          </div>
          <details className="device-diagnostics">
            <summary>
              <Icon name="Activity" size={14} />
              {t('npu.diagnostics')}
              <span>{t('npu.troubleshooting')}</span>
            </summary>
            <div className="diagnostic-grid">
              <div>
                <span>{t('npu.rawSample')}</span>
                <strong>{topology.load_percent === null ? '—' : `${number(topology.load_percent)}%`}</strong>
                <small>{t('npu.sampleHint')}</small>
              </div>
              <div>
                <span>{t('npu.perCoreSample')}</span>
                <strong>{hasCoreLoad(telemetry.npu_core_load_percent) ? t('common.available') : t('common.unavailable')}</strong>
                <small>{t('npu.debugfsHint')}</small>
              </div>
              <div>
                <span>{t('npu.driver')}</span>
                <strong>{topology.driver}</strong>
                <small>{topology.driver_version ?? t('npu.unknownVersion')}</small>
              </div>
              <div>
                <span>{t('detail.rknnRuntime')}</span>
                <strong>{shortVersion(topology.runtime_version)}</strong>
                <small>{t('npu.environment')}</small>
              </div>
            </div>
          </details>
        </section>

        <aside className="scheduler-dock" aria-label={t('npu.leaseScheduling')}>
          <div className="scheduler-tabs" role="group" aria-label={t('npu.schedulerTabs')}>
            <button
              type="button"
              className={schedulerView === 'available' ? 'active' : ''}
              aria-pressed={schedulerView === 'available'}
              onClick={() => setSchedulerView('available')}
            >
              {t('npu.schedulable')} <span>{number(candidates.length)}</span>
            </button>
            <button
              type="button"
              className={schedulerView === 'leases' ? 'active' : ''}
              aria-pressed={schedulerView === 'leases'}
              onClick={() => setSchedulerView('leases')}
            >
              {t('npu.activeLeases')} <span>{number(allocations.length)}</span>
            </button>
          </div>
          <div className="scheduler-panel">
            {schedulerView === 'available' ? (
              candidates.length ? (
                <div className="dock-list">
                  {candidates.map((plugin) => (
                    <div className="dock-candidate" key={plugin.id}>
                      <Link to={`/plugins/${plugin.id}`}>
                        <strong>{plugin.name}</strong>
                        <span>
                          {plugin.id} · {pluginStateLabel(plugin.state)}
                        </span>
                      </Link>
                      <div className="mask-actions">
                        {masks()
                          .filter((mask) => plugin.allowed_masks.includes(mask.value))
                          .map((mask) => (
                            <button
                              type="button"
                              key={mask.value}
                              disabled={actionId === plugin.id}
                              onClick={() => void allocate(plugin.id, mask.value)}
                            >
                              {mask.label}
                            </button>
                          ))}
                      </div>
                    </div>
                  ))}
                </div>
              ) : (
                <div className="dock-empty">
                  <Icon name="Check" size={16} />
                  <span>{t('npu.noPending')}</span>
                </div>
              )
            ) : allocations.length ? (
              <div className="dock-list">
                {allocations.map((allocation) => (
                  <div className="dock-lease" key={allocation.lease_id}>
                    <div>
                      <span className="lease-status">
                        <Icon name="Check" size={12} />
                      </span>
                      <Link to={`/plugins/${allocation.plugin_id}`}>{allocation.plugin_id}</Link>
                      <strong>{maskLabel(allocation.core_mask)}</strong>
                    </div>
                    <code>{allocation.lease_id}</code>
                    <button
                      type="button"
                      className="release-button"
                      onClick={() => void release(allocation.lease_id)}
                      disabled={actionId === allocation.lease_id}
                    >
                      {actionId === allocation.lease_id ? (
                        <Icon name="LoaderCircle" size={13} className="spin" />
                      ) : (
                        <Icon name="Power" size={13} />
                      )}
                      {t('npu.release')}
                    </button>
                  </div>
                ))}
              </div>
            ) : (
              <div className="dock-empty">
                <Icon name="Cpu" size={16} />
                <span>{t('npu.noLeases')}</span>
              </div>
            )}
          </div>
          <details className="scheduler-policy">
            <summary>
              <Icon name="ServerCog" size={14} />
              {t('npu.policy')}
            </summary>
            <div>
              <p>{t('npu.policyHint')}</p>
            </div>
          </details>
        </aside>
      </div>
    </>
  )
}

function fillHeight(allocated: boolean, load: number | null | undefined) {
  if (typeof load === 'number') return Math.max(allocated ? 12 : 0, Math.min(100, load))
  return allocated ? 36 : 0
}

function formatFrequency(value: number | null) {
  return value === null ? '—' : value >= 1e9 ? `${number(value / 1e9, 2)} GHz` : `${number(Math.round(value / 1e6))} MHz`
}
function formatTemperature(value: number | null) {
  return value === null ? '—' : `${number(value, 1)} °C`
}
function formatMemory(total: number | null, available: number | null) {
  if (total === null || available === null || total <= 0) return '—'
  const gib = 1024 ** 3
  return `${number((total - available) / gib, 1)} / ${number(total / gib, 1)} GiB`
}
function formatMemoryPercent(total: number | null, available: number | null) {
  if (total === null || available === null || total <= 0) return t('npu.memoryUnavailable')
  return t('npu.memoryPercent', { percent: number(Math.round(((total - available) / total) * 100)) })
}
function hasCoreLoad(values: Array<number | null>) {
  return values.some((value) => value !== null)
}
function coreStatus(allocated: boolean, load: number | null | undefined) {
  const state = allocated ? t('npu.occupied') : t('npu.idle')
  return load === null || load === undefined ? state : `${state} · ${number(load)}%`
}
function shortVersion(value: string | null) {
  return value?.split(' ')[0] ?? '—'
}
function maskLabel(mask: CoreMask) {
  return mask === 'core0_1' ? t('npu.dualCore') : mask === 'core0' ? t('npu.coreZero') : mask === 'core1' ? t('npu.coreOne') : t('common.auto')
}
function pluginStateLabel(state: string) {
  return state === 'installed' ? t('common.unallocated') : state === 'failed' ? t('common.fault') : state === 'backoff' ? t('common.restarting') : state
}
