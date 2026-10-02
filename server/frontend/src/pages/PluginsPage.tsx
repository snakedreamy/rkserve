import { t, number } from '../i18n/core'
import { useI18n } from '../i18n/useI18n'
import { Icon } from '../icons/Icon'
import { Link } from 'react-router-dom'
import { useSystem } from '../app/SystemContext'
import { PageHeading } from '../components/PageHeading'

const stateLabels = (): Record<string, string> => ({
  installed: t('common.unallocated'),
  ready: t('common.running'),
  backoff: t('common.restarting'),
  failed: t('common.fault'),
})

export function PluginsPage() {
  useI18n()
  const { plugins, workers, loading } = useSystem()
  const runningCount = workers.filter((worker) => worker.status === 'ready').length
  return (
    <>
      <PageHeading
        eyebrow={t('plugins.eyebrow')}
        title={t('plugins.title')}
        description={t('plugins.counts', { running: number(runningCount), installed: number(plugins.length) })}
        aside={
          <div className="heading-state">
            <span>{number(plugins.length)}</span>
            <small>{t('common.installed')}</small>
          </div>
        }
      />
      <section className="section-block">
        {loading ? (
          <div className="empty-state">
            <Icon name="LoaderCircle" size={16} className="spin" />
            {t('plugins.loading')}
          </div>
        ) : plugins.length === 0 ? (
          <div className="empty-state">
            <Icon name="Box" size={22} />
            <div>
              <strong>{t('plugins.empty')}</strong>
              <span>{t('plugins.emptyHint')}</span>
            </div>
          </div>
        ) : (
          <div className="plugin-grid">
            {plugins.map((plugin) => {
              const worker = workers.find((item) => item.plugin_id === plugin.id)
              const ready = worker?.status === 'ready'
              const state = stateLabels()[worker?.status ?? plugin.state] ?? plugin.state
              return (
                <article className={`plugin-card ${ready ? 'ready' : plugin.state}`} key={plugin.id}>
                  <header className="plugin-card-head">
                    <span className={`service-status ${ready ? 'ready' : ''}`} />
                    <span className={`badge ${ready ? 'ready' : worker?.status === 'backoff' ? 'busy' : ''}`}>
                      {state}
                    </span>
                  </header>
                  <h3>{plugin.name}</h3>
                  <p className="mono">
                    {plugin.id} · v{plugin.version}
                  </p>
                  <ul className="cap-chips">
                    {plugin.capabilities.slice(0, 3).map((capability) => (
                      <li key={capability.id}>{capability.name}</li>
                    ))}
                    {plugin.capabilities.length > 3 && <li>+{plugin.capabilities.length - 3}</li>}
                    {plugin.capabilities.length === 0 && <li>{t('plugins.noCapabilities')}</li>}
                  </ul>
                  <footer className="plugin-card-actions">
                    <Link className="detail-link" to={`/plugins/${plugin.id}`}>
                      {t('common.details')}
                      <Icon name="ChevronRight" size={14} />
                    </Link>
                    {ready ? (
                      <Link className="detail-link" to={`/plugins/${plugin.id}/run`}>
                        {t('common.run')}
                        <Icon name="Play" size={13} />
                      </Link>
                    ) : (
                      <Link className="detail-link" to="/npu">
                        {t('plugins.allocate')}
                        <Icon name="CircleGauge" size={13} />
                      </Link>
                    )}
                  </footer>
                </article>
              )
            })}
          </div>
        )}
      </section>
    </>
  )
}
