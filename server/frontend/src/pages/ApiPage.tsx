import { t, number, UiError, errorText, localizePlugin } from '../i18n/core'
import { useI18n } from '../i18n/useI18n'
import { Icon } from '../icons/Icon'
import { useEffect, useMemo, useState } from 'react'
import { api } from '../api'
import { useSystem } from '../app/SystemContext'
import { PageHeading } from '../components/PageHeading'
import type { PluginSpec } from '../types'

export function ApiPage() {
  useI18n()
  const { plugins } = useSystem()
  const [specs, setSpecs] = useState<PluginSpec[]>([])
  const [error, setError] = useState<UiError | null>(null)
  const [loading, setLoading] = useState(true)
  const [selectedKey, setSelectedKey] = useState('')
  const [copied, setCopied] = useState(false)
  const [copyError, setCopyError] = useState(false)
  const pluginSignature = plugins.map((plugin) => `${plugin.id}:${plugin.version}`).join('|')

  useEffect(() => {
    let disposed = false
    if (!plugins.length) {
      setSpecs([])
      setLoading(false)
      return
    }
    setLoading(true)
    void Promise.allSettled(plugins.map((plugin) => api.pluginSpec(plugin.id))).then((results) => {
      if (disposed) return
      const nextSpecs = results.flatMap((result) => result.status === 'fulfilled' ? [result.value] : [])
      const failedCount = results.length - nextSpecs.length
      setSpecs(nextSpecs)
      setError(failedCount ? new UiError('api.partialFailure', { count: failedCount }) : null)
    }).finally(() => {
      if (!disposed) setLoading(false)
    })
    return () => { disposed = true }
  }, [pluginSignature])

  const capabilities = specs.flatMap((spec) => {
    const plugin = localizePlugin({ ...spec.plugin, capabilities: spec.jobs.capabilities })
    return plugin.capabilities.map((capability) => ({ spec: { ...spec, plugin }, capability }))
  })
  const selected = capabilities.find((item) => `${item.spec.plugin.id}:${item.capability.id}` === selectedKey) ?? capabilities[0]
  const endpoint = selected?.spec.jobs.submit.path.replace('{capability_id}', selected.capability.id)
  const example = useMemo(() => selected && endpoint ? (() => {
    const query = new URLSearchParams(selected.capability.parameters.flatMap((parameter) => parameter.default ? [[parameter.key, parameter.default]] : []))
    const body = selected.capability.input_kind === 'text'
      ? "  --data-binary 'Hello from RKServe'"
      : `  --data-binary '@input.${selected.capability.input_kind === 'image' ? 'jpg' : selected.capability.input_kind === 'audio' ? 'wav' : 'bin'}'`
    return [
      `curl -X POST https://rkserve.example.com${endpoint}${query.size ? `?${query}` : ''} \\`,
      `  -H "X-API-Key: \${RKSERVE_API_KEY}" \\`,
      `  -H 'Content-Type: ${selected.capability.accepted_content_types[0]}' \\`,
      body,
    ].join('\n')
  })() : '', [endpoint, selected])

  async function copyExample() {
    setCopyError(false)
    try {
      await navigator.clipboard.writeText(example)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1600)
    } catch {
      setCopyError(true)
    }
  }

  return (
    <>
      <PageHeading eyebrow={t('common.apiTerm')} title={t('api.title')} description={t('api.description')} />
      {error && <div className="error-strip"><Icon name="CircleAlert" size={14} />{errorText(error)}</div>}
      {copyError && <div className="error-strip" role="alert"><Icon name="CircleAlert" size={14} />{t('error.copy')}</div>}
      <div className="api-workspace">
        <section className="section-block">
          <div className="section-heading"><div><span className="eyebrow">{t('api.endpoints')}</span><h2>{t('api.capabilities')}</h2></div><span className="section-count">{t('api.count', { count: number(capabilities.length) })}</span></div>
          {loading ? <div className="empty-state"><Icon name="LoaderCircle" size={16} className="spin" />{t('api.loading')}</div> : capabilities.length === 0 ? (
            <div className="empty-state"><Icon name="Braces" size={20} /><div><strong>{t('api.empty')}</strong><span>{t('api.emptyHint')}</span></div></div>
          ) : capabilities.map(({ spec, capability }) => {
            const key = `${spec.plugin.id}:${capability.id}`
            const path = spec.jobs.submit.path.replace('{capability_id}', capability.id)
            return <button type="button" className={`api-row ${selected && `${selected.spec.plugin.id}:${selected.capability.id}` === key ? 'selected' : ''}`} key={key} onClick={() => setSelectedKey(key)}><span className="method">{spec.jobs.submit.method}</span><code>{path}</code><span>{spec.plugin.name} · {capability.name}</span><Icon name="Braces" size={15} /></button>
          })}
        </section>
        {selected && endpoint && <section className="section-block api-example">
          <div className="section-heading"><div><span className="eyebrow">{t('api.example')}</span><h2>{selected.capability.name}</h2></div><button type="button" className="icon-button" aria-label={t('api.copyExample')} title={t('api.copyExample')} onClick={() => void copyExample()}><Icon name={copied ? 'Check' : 'Copy'} size={14} /></button></div>
          <pre><code>{example}</code></pre>
          <div className="api-job-flow">
            <span className="eyebrow">{t('api.flow')}</span>
            <div className={`method-${selected.spec.jobs.submit.method.toLowerCase()}`}><strong>{selected.spec.jobs.submit.method}</strong><code>{endpoint}</code><span>{t('run.submit')}</span></div>
            <div className={`method-${selected.spec.jobs.get.method.toLowerCase()}`}><strong>{selected.spec.jobs.get.method}</strong><code>{selected.spec.jobs.get.path}</code><span>{t('api.getStatus')}</span></div>
            <div className={`method-${selected.spec.jobs.result.method.toLowerCase()}`}><strong>{selected.spec.jobs.result.method}</strong><code>{selected.spec.jobs.result.path}</code><span>{t('api.getResult')}</span></div>
            <div className={`method-${selected.spec.jobs.cancel.method.toLowerCase()}`}><strong>{selected.spec.jobs.cancel.method}</strong><code>{selected.spec.jobs.cancel.path}</code><span>{t('job.cancel')}</span></div>
          </div>
        </section>}
      </div>
    </>
  )
}
