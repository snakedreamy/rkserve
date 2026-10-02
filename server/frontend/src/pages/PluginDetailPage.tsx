import { t, number } from '../i18n/core'
import { useI18n } from '../i18n/useI18n'
import { Icon } from '../icons/Icon'
import { useEffect, useState } from 'react'
import { Link, Navigate, useParams } from 'react-router-dom'
import { api } from '../api'
import { useSystem } from '../app/SystemContext'
import { ConfigField } from '../components/ConfigField'
import { PageHeading } from '../components/PageHeading'

const workerStateLabels = (): Record<string, string> => ({
  ready: t('common.running'),
  backoff: t('common.restarting'),
})

export function PluginDetailPage() {
  useI18n()
  const { pluginId = '' } = useParams()
  const { plugins, allocations, workers, loading, actionId, runAction } = useSystem()
  const plugin = plugins.find((item) => item.id === pluginId)
  const [configuration, setConfiguration] = useState<Record<string, string>>({})
  const [activeSection, setActiveSection] = useState<'overview' | 'configuration'>('overview')
  const configurationSignature = JSON.stringify(plugin?.configuration_values ?? {})
  const activeModel = plugin?.configuration_values.model

  useEffect(() => {
    if (plugin) setConfiguration(plugin.configuration_values)
  }, [plugin?.id, configurationSignature])

  if (!plugin && !loading) return <Navigate to="/plugins" replace />
  if (!plugin) return null
  const allocation = allocations.find((item) => item.plugin_id === plugin.id)
  const worker = workers.find((item) => item.plugin_id === plugin.id)
  const configDirty = JSON.stringify(configuration) !== JSON.stringify(plugin.configuration_values)

  async function saveConfiguration() {
    await runAction(`config:${plugin!.id}`, () => api.configure(plugin!.id, configuration))
  }

  const configurationAction = actionId === `config:${plugin.id}`

  return (
    <>
      <Link className="back-link" to="/plugins"><Icon name="ArrowLeft" size={14} />{t('detail.back')}</Link>
      <PageHeading
        eyebrow={t('detail.eyebrow')}
        title={plugin.name}
        description={`${plugin.id} · v${plugin.version}`}
        aside={<span className={`state-label ${worker?.status === 'ready' ? 'ready' : ''}`}>{worker ? workerStateLabels()[worker.status] ?? worker.status : t('common.unallocated')}</span>}
      />
      <div className="detail-tabs" role="tablist" aria-label={t('detail.tabs')}>
        <button type="button" role="tab" aria-selected={activeSection === 'overview'} className={activeSection === 'overview' ? 'active' : ''} onClick={() => setActiveSection('overview')}>
          {t('detail.overview')}
          <span>{t('detail.capabilityCount', { count: number(plugin.capabilities.length) })}</span>
        </button>
        <button type="button" role="tab" aria-selected={activeSection === 'configuration'} className={activeSection === 'configuration' ? 'active' : ''} onClick={() => setActiveSection('configuration')}>
          {t('detail.startupConfig')}
          <span>{plugin.configuration.length ? t('detail.configurationCount', { count: number(plugin.configuration.length) }) : t('detail.noConfig')}</span>
        </button>
      </div>
      <div className="detail-layout">
        <div className="detail-main">
          {activeSection === 'overview' ? (
            <div role="tabpanel" className="detail-tab-panel">
              <section className="detail-section">
                <span className="eyebrow">{t('detail.serviceCapabilities')}</span><h2>{t('detail.capabilities')}</h2>
                <div className="capability-list">{plugin.capabilities.map((capability) => <div className="capability-row" key={capability.id}><div><strong>{capability.name}</strong><span>{capability.description}</span></div><code>{capability.id}</code><span>{t(`kind.${capability.input_kind}`)} → {t(`kind.${capability.output_kind}`)}</span></div>)}</div>
              </section>
              <section className="detail-section"><span className="eyebrow">{t('detail.constraints')}</span><h2>{t('detail.scheduling')}</h2><dl className="detail-list"><div><dt>{t('detail.defaultCore')}</dt><dd>{plugin.default_mask}</dd></div><div><dt>{t('detail.allowedCores')}</dt><dd>{plugin.allowed_masks.join(' / ')}</dd></div><div><dt>{t('detail.concurrency')}</dt><dd>{number(plugin.max_concurrency)}</dd></div><div><dt>{t('detail.queue')}</dt><dd>{number(plugin.queue_size)}</dd></div><div><dt>{t('detail.timeout')}</dt><dd>{number(plugin.request_timeout_ms)} ms</dd></div></dl></section>
            </div>
          ) : (
            <section className="detail-section detail-tab-panel" role="tabpanel">
              <div className="section-heading"><div><span className="eyebrow">{t('detail.loadOptions')}</span><h2>{t('detail.configuration')}</h2></div>{plugin.configuration.length > 0 && <button className="text-button primary" type="button" disabled={!configDirty || configurationAction} onClick={() => void saveConfiguration()}>{configurationAction ? <Icon name="LoaderCircle" size={14} className="spin" /> : <Icon name="RefreshCw" size={14} />}{configurationAction ? t('detail.reloading') : worker ? t('detail.applyReload') : t('detail.save')}</button>}</div>
              {plugin.configuration.length ? (
                <div className="configuration-form">
                  {plugin.configuration.map((field) => (
                    <label key={field.key}>
                      <div><strong>{field.label}</strong><span>{field.description}</span></div>
                      <ConfigField
                        field={field}
                        value={configuration[field.key] ?? field.default ?? ''}
                        disabled={configurationAction}
                        emptyLabel={t('common.notSet')}
                        placeholder={field.required ? t('common.required') : t('detail.workerDefault')}
                        onChange={(value) => setConfiguration((current) => ({ ...current, [field.key]: value }))}
                      />
                    </label>
                  ))}
                </div>
              ) : <p className="muted-copy">{t('detail.noEditableConfig')}</p>}
              {worker && plugin.configuration.length > 0 && <p className="implementation-note">{t('detail.reloadHint')}</p>}
            </section>
          )}
        </div>
        <aside className="detail-aside">
          <span className="eyebrow">{t('detail.runtime')}</span><h2>{worker ? workerStateLabels()[worker.status] ?? worker.status : t('detail.notStarted')}</h2>
          <dl className="detail-list">
            <div><dt>{t('detail.pid')}</dt><dd>{worker?.pid ?? '—'}</dd></div>
            <div><dt>{t('detail.rknnRuntime')}</dt><dd>{worker?.runtime_version.split(' ')[0] ?? '—'}</dd></div>
            <div><dt>{t('detail.driverVersion')}</dt><dd>{worker?.driver_version ?? '—'}</dd></div>
            <div><dt>{t('detail.occupiedCores')}</dt><dd>{allocation?.core_mask ?? '—'}</dd></div>
            {activeModel && <div><dt>{t('detail.currentModel')}</dt><dd title={activeModel}>{modelDisplayName(activeModel)}</dd></div>}
            <div><dt>{t('detail.capabilityTotal')}</dt><dd>{number(plugin.capabilities.length)}</dd></div>
          </dl>
          <div className="detail-actions">
            {worker?.status === 'ready' ? (
              <Link className="action-button" to={`/plugins/${plugin.id}/run`}><Icon name="ScanSearch" size={15} />{t('detail.run')}</Link>
            ) : (
              <Link className="action-button" to="/npu"><Icon name="CircleGauge" size={15} />{t('detail.allocateNpu')}</Link>
            )}
            <Link className="action-button secondary" to="/npu"><Icon name="ServerCog" size={15} />{t('detail.leases')}</Link>
          </div>
        </aside>
      </div>
    </>
  )
}

function modelDisplayName(model: string) {
  return model.split('/').at(-1)?.replace(/\.rknn$/i, '') ?? model
}
