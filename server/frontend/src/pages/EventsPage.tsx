import { t, number, dateTime, time, UiError, errorText } from '../i18n/core'
import { useI18n } from '../i18n/useI18n'
import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from 'react'
import { Icon } from '../icons/Icon'
import { api } from '../api'
import { useSystem } from '../app/SystemContext'
import { PageHeading } from '../components/PageHeading'
import type { EventFilters, EventSeverity, RuntimeEvent } from '../types'

const severityLabels = (): Record<EventSeverity, string> => ({
  info: t('events.info'),
  warning: t('events.warning'),
  error: t('common.error'),
})

const categoryLabels = (): Record<string, string> => ({
  system: t('events.system'),
  worker: t('common.worker'),
  supervisor: t('events.supervisor'),
  scheduler: t('events.scheduler'),
  plugin: t('events.plugin'),
  job: t('events.job'),
  api: t('common.apiTerm'),
  auth: t('events.auth'),
})

const moduleOptions = () => [
  { value: '/api/v1/auth', label: t('events.auth') },
  { value: '/api/v1/plugins', label: t('events.plugin') },
  { value: '/api/v1/scheduler', label: t('events.scheduling') },
  { value: '/api/v1/npu', label: t('common.npu') },
  { value: '/api/v1/workers', label: t('common.worker') },
  { value: '/api/v1/system', label: t('events.system') },
  { value: '/api/v1/events', label: t('events.audit') },
]

const kindLabels = (): Record<string, string> => ({
  request_completed: t('events.requestCompleted'),
  request_failed: t('events.requestFailed'),
  authentication_required: t('events.unauthenticated'),
  role_denied: t('events.roleDenied'),
  scope_denied: t('events.scopeDenied'),
})

function mergeEvents(...eventLists: RuntimeEvent[][]) {
  const events = new Map<number, RuntimeEvent>()
  for (const list of eventLists) {
    for (const event of list) events.set(event.id, event)
  }
  return [...events.values()].sort((left, right) => right.timestamp_unix_ms - left.timestamp_unix_ms || right.id - left.id)
}

function isFailure(event: RuntimeEvent) {
  if (event.http_status !== null) return event.http_status >= 400
  return event.severity === 'error'
}

function formatTime(ms: number) {
  return dateTime(ms)
}

function formatDuration(durationUs: number | null) {
  if (durationUs === null) return '—'
  const ms = durationUs / 1000
  return ms >= 1000 ? `${number(ms / 1000, 3)} s` : `${number(ms, 1)} ms`
}

function formatBytes(value: number | null) {
  if (value === null) return '—'
  if (value < 1024) return `${number(value)} B`
  return `${number(value / 1024, 1)} KB`
}

function clip(value: string | null | undefined, max = 48) {
  if (!value) return '—'
  return value.length > max ? `${value.slice(0, max)}…` : value
}

function metaString(event: RuntimeEvent, key: string) {
  const value = event.metadata?.[key]
  return typeof value === 'string' ? value : null
}

function metaList(event: RuntimeEvent, key: string) {
  const value = event.metadata?.[key]
  return Array.isArray(value) ? value.map(String) : []
}

function eventError(event: RuntimeEvent) {
  return metaString(event, 'error')
}

function auditModule(event: RuntimeEvent) {
  const path = event.http_path ?? ''
  const match = path.match(/^\/api\/v1\/([^/?]+)/)
  const found = moduleOptions().find((option) => option.value === `/api/v1/${match?.[1] ?? ''}`)
  return found?.label ?? categoryLabels()[event.category] ?? event.category
}

function auditAction(event: RuntimeEvent) {
  return kindLabels()[event.kind] ?? event.kind
}

function roleLabel(role: string | null) {
  if (role === 'management') return t('common.management')
  if (role === 'api') return t('common.api')
  return role ?? '—'
}

export function EventsPage() {
  useI18n()
  const { plugins } = useSystem()
  const [view, setView] = useState<'audit' | 'runtime'>('audit')
  const [events, setEvents] = useState<RuntimeEvent[]>([])
  const eventsRef = useRef<RuntimeEvent[]>([])
  const [nextCursor, setNextCursor] = useState<number | null>(null)
  const [searchInput, setSearchInput] = useState('')
  const [q, setQ] = useState('')
  const [outcome, setOutcome] = useState<'' | 'success' | 'failure'>('')
  const [httpMethod, setHttpMethod] = useState('')
  const [httpStatus, setHttpStatus] = useState('')
  const [pathPrefix, setPathPrefix] = useState('')
  const [kind, setKind] = useState('')
  const [severity, setSeverity] = useState<EventSeverity | ''>('')
  const [category, setCategory] = useState('')
  const [pluginId, setPluginId] = useState('')
  const [actor, setActor] = useState('')
  const [clientIp, setClientIp] = useState('')
  const [from, setFrom] = useState('')
  const [to, setTo] = useState('')
  const [pageSize, setPageSize] = useState(15)
  const [selectedId, setSelectedId] = useState<number | null>(null)
  const [loading, setLoading] = useState(true)
  const [refreshing, setRefreshing] = useState(false)
  const [loadingMore, setLoadingMore] = useState(false)
  const [error, setError] = useState<unknown>(null)
  const [updatedAt, setUpdatedAt] = useState<Date | null>(null)

  const filters = useMemo<EventFilters>(() => ({
    q: q || undefined,
    outcome: outcome || undefined,
    severity: view === 'runtime' ? (severity || undefined) : undefined,
    category: view === 'runtime' ? (category || undefined) : undefined,
    kind: kind || undefined,
    pluginId: pluginId || undefined,
    actor: actor || undefined,
    clientIp: clientIp || undefined,
    httpMethod: view === 'audit' ? (httpMethod || undefined) : undefined,
    httpStatus: httpStatus ? Number(httpStatus) : undefined,
    pathPrefix: view === 'audit' ? (pathPrefix || undefined) : undefined,
    source: view === 'audit' ? 'http' : undefined,
    excludeSource: view === 'runtime' ? 'http' : undefined,
    from: from ? new Date(from).getTime() : undefined,
    to: to ? new Date(to).getTime() : undefined,
    limit: pageSize,
  }), [actor, category, clientIp, from, httpMethod, httpStatus, kind, outcome, pageSize, pathPrefix, pluginId, q, severity, to, view])

  const reloadEvents = useCallback(async () => {
    setLoading(true)
    try {
      const response = await api.events(filters)
      eventsRef.current = response.items
      setEvents(response.items)
      setNextCursor(response.next_cursor)
      setUpdatedAt(new Date())
      setError(null)
    } catch (cause) {
      setError(cause instanceof Error ? cause : new UiError('error.events'))
    } finally {
      setLoading(false)
    }
  }, [filters])

  const refreshEvents = useCallback(async () => {
    const latestEvent = eventsRef.current[0]
    if (!latestEvent) return reloadEvents()
    setRefreshing(true)
    try {
      const response = await api.events({ ...filters, after: latestEvent.id })
      const nextEvents = mergeEvents(response.items, eventsRef.current)
      eventsRef.current = nextEvents
      setEvents(nextEvents)
      setUpdatedAt(new Date())
      setError(null)
    } catch (cause) {
      setError(cause instanceof Error ? cause : new UiError('error.events'))
    } finally {
      setRefreshing(false)
    }
  }, [filters, reloadEvents])

  const loadMore = useCallback(async () => {
    if (!nextCursor) return
    setLoadingMore(true)
    try {
      const response = await api.events({ ...filters, cursor: nextCursor })
      const nextEvents = mergeEvents(eventsRef.current, response.items)
      eventsRef.current = nextEvents
      setEvents(nextEvents)
      setNextCursor(response.next_cursor)
      setUpdatedAt(new Date())
      setError(null)
    } catch (cause) {
      setError(cause instanceof Error ? cause : new UiError('error.events'))
    } finally {
      setLoadingMore(false)
    }
  }, [filters, nextCursor])

  useEffect(() => {
    void reloadEvents()
  }, [reloadEvents])

  useEffect(() => {
    const timer = window.setInterval(() => void refreshEvents(), 3_000)
    return () => window.clearInterval(timer)
  }, [refreshEvents])

  const selected = events.find((event) => event.id === selectedId) ?? null
  const failedCount = events.filter(isFailure).length

  function switchView(next: 'audit' | 'runtime') {
    if (next === view) return
    setView(next)
    setSelectedId(null)
    setKind('')
    setPathPrefix('')
    eventsRef.current = []
    setEvents([])
    setNextCursor(null)
  }

  function applySearch(event: FormEvent) {
    event.preventDefault()
    setQ(searchInput.trim())
  }

  function resetFilters() {
    setSearchInput('')
    setQ('')
    setOutcome('')
    setHttpMethod('')
    setHttpStatus('')
    setPathPrefix('')
    setKind('')
    setSeverity('')
    setCategory('')
    setPluginId('')
    setActor('')
    setClientIp('')
    setFrom('')
    setTo('')
    setPageSize(15)
    setSelectedId(null)
  }

  return (
    <>
      <PageHeading
        eyebrow={t('events.eyebrow')}
        title={view === 'audit' ? t('events.apiAudit') : t('events.runtime')}
        description={view === 'audit' ? t('events.auditDescription') : t('events.runtimeDescription')}
        aside={<div className="heading-state"><span>{number(events.length)}</span><small>{t('events.currentResults')}</small></div>}
      />

      <div className="seg-group event-view-toggle" role="tablist" aria-label={t('events.views')}>
        <button type="button" role="tab" aria-selected={view === 'audit'} className={`seg-item ${view === 'audit' ? 'active' : ''}`} onClick={() => switchView('audit')}>{t('events.apiAudit')}</button>
        <button type="button" role="tab" aria-selected={view === 'runtime'} className={`seg-item ${view === 'runtime' ? 'active' : ''}`} onClick={() => switchView('runtime')}>{t('events.runtime')}</button>
      </div>

      <section className="section-block">
        <div className="section-heading">
          <div>
            <span className="eyebrow">{view === 'audit' ? t('events.requests') : t('events.timeline')}</span>
            <h2>{view === 'audit' ? t('events.auditLog') : t('events.runtime')}</h2>
          </div>
          <span className="section-count">{t('events.summary', { count: number(failedCount), updated: updatedAt ? t('events.updated', { time: time(updatedAt) }) : t('events.waitingQuery') })}</span>
        </div>

        <form className="log-toolbar" onSubmit={applySearch} aria-label={t('events.filters')}>
          <label className="log-search">
            <span>{t('events.search')}</span>
            <input
              value={searchInput}
              onChange={(event) => setSearchInput(event.target.value)}
              placeholder={view === 'audit' ? t('events.auditSearch') : t('events.runtimeSearch')}
            />
          </label>
          <label>
            <span>{t('events.outcome')}</span>
            <select value={outcome} onChange={(event) => setOutcome(event.target.value as '' | 'success' | 'failure')}>
              <option value="">{t('common.all')}</option>
              <option value="success">{t('common.success')}</option>
              <option value="failure">{t('common.failed')}</option>
            </select>
          </label>
          {view === 'audit' ? (
            <>
              <label>
                <span>{t('events.httpStatus')}</span>
                <select value={httpStatus} onChange={(event) => setHttpStatus(event.target.value)}>
                  <option value="">{t('common.all')}</option>
                  <option value="200">200</option>
                  <option value="202">202</option>
                  <option value="204">204</option>
                  <option value="400">400</option>
                  <option value="401">401</option>
                  <option value="403">403</option>
                  <option value="404">404</option>
                  <option value="409">409</option>
                  <option value="500">500</option>
                </select>
              </label>
              <label>
                <span>{t('events.businessModule')}</span>
                <select value={pathPrefix} onChange={(event) => setPathPrefix(event.target.value)}>
                  <option value="">{t('common.all')}</option>
                  {moduleOptions().map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
                </select>
              </label>
              <label>
                <span>{t('events.action')}</span>
                <select value={kind} onChange={(event) => setKind(event.target.value)}>
                  <option value="">{t('common.all')}</option>
                  <option value="request_completed">{t('events.requestCompleted')}</option>
                  <option value="request_failed">{t('events.requestFailed')}</option>
                  <option value="authentication_required">{t('events.unauthenticated')}</option>
                  <option value="role_denied">{t('events.roleDenied')}</option>
                  <option value="scope_denied">{t('events.scopeDenied')}</option>
                </select>
              </label>
              <label>
                <span>{t('events.method')}</span>
                <select value={httpMethod} onChange={(event) => setHttpMethod(event.target.value)}>
                  <option value="">{t('common.all')}</option>
                  <option value="GET">GET</option>
                  <option value="POST">POST</option>
                  <option value="PUT">PUT</option>
                  <option value="DELETE">DELETE</option>
                </select>
              </label>
            </>
          ) : (
            <>
              <label>
                <span>{t('events.severity')}</span>
                <select value={severity} onChange={(event) => setSeverity(event.target.value as EventSeverity | '')}>
                  <option value="">{t('common.all')}</option>
                  <option value="info">{t('events.info')}</option>
                  <option value="warning">{t('events.warning')}</option>
                  <option value="error">{t('common.error')}</option>
                </select>
              </label>
              <label>
                <span>{t('events.source')}</span>
                <select value={category} onChange={(event) => setCategory(event.target.value)}>
                  <option value="">{t('common.all')}</option>
                  <option value="system">{t('events.system')}</option>
                  <option value="worker">{t('common.worker')}</option>
                  <option value="supervisor">{t('events.supervisor')}</option>
                  <option value="scheduler">{t('events.scheduler')}</option>
                  <option value="plugin">{t('events.plugin')}</option>
                  <option value="job">{t('events.job')}</option>
                </select>
              </label>
            </>
          )}
          <label>
            <span>{t('events.pageSize')}</span>
            <select value={pageSize} onChange={(event) => setPageSize(Number(event.target.value))}>
              <option value={15}>15</option>
              <option value={50}>50</option>
              <option value={100}>100</option>
            </select>
          </label>
          <div className="log-toolbar-actions">
            <button type="submit" className="action-button">{t('events.query')}</button>
            <button type="button" className="text-button primary" onClick={resetFilters}>{t('events.reset')}</button>
            <button type="button" className="icon-button" onClick={() => void refreshEvents()} disabled={loading || refreshing} title={t('common.refresh')} aria-label={t('common.refresh')}>
              <Icon name="RefreshCw" size={14} className={loading || refreshing ? 'spin' : ''} />
            </button>
          </div>
          <details className="log-advanced">
            <summary>{t('events.moreFilters')}</summary>
            <div>
              <label>
                <span>{t('events.caller')}</span>
                <input value={actor} onChange={(event) => setActor(event.target.value)} placeholder={t('events.callerExample')} />
              </label>
              <label>
                <span>{t('events.clientIp')}</span>
                <input value={clientIp} onChange={(event) => setClientIp(event.target.value)} placeholder={t('events.ipExample')} />
              </label>
              <label>
                <span>{t('events.plugin')}</span>
                <select value={pluginId} onChange={(event) => setPluginId(event.target.value)}>
                  <option value="">{t('common.all')}</option>
                  {plugins.map((plugin) => <option key={plugin.id} value={plugin.id}>{plugin.name}</option>)}
                </select>
              </label>
              <label>
                <span>{t('events.startTime')}</span>
                <input type="datetime-local" value={from} onChange={(event) => setFrom(event.target.value)} />
              </label>
              <label>
                <span>{t('events.endTime')}</span>
                <input type="datetime-local" value={to} onChange={(event) => setTo(event.target.value)} />
              </label>
            </div>
          </details>
        </form>

        {Boolean(error) && <div className="error-strip"><Icon name="CircleAlert" size={14} />{errorText(error)}</div>}

        <div className="log-table-wrap">
          <table className="log-table">
            <thead>
              {view === 'audit' ? (
                <tr>
                  <th className="log-col-id">{t('common.id')}</th>
                  <th>{t('events.time')}</th>
                  <th>{t('events.outcome')}</th>
                  <th>{t('events.statusCode')}</th>
                  <th>{t('events.module')}</th>
                  <th>{t('events.action')}</th>
                  <th>{t('common.api')}</th>
                  <th>{t('events.caller')}</th>
                  <th className="log-col-ip">{t('events.client')}</th>
                  <th>{t('events.elapsed')}</th>
                  <th className="log-col-error">{t('common.error')}</th>
                  <th className="log-col-trace">{t('events.requestId')}</th>
                </tr>
              ) : (
                <tr>
                  <th className="log-col-id">{t('common.id')}</th>
                  <th>{t('events.time')}</th>
                  <th>{t('events.severity')}</th>
                  <th>{t('events.source')}</th>
                  <th>{t('events.event')}</th>
                  <th>{t('events.message')}</th>
                  <th>{t('events.related')}</th>
                </tr>
              )}
            </thead>
            <tbody>
              {events.length ? events.map((event) => {
                const failed = isFailure(event)
                const selectedRow = event.id === selectedId
                if (view === 'audit') {
                  return (
                    <tr
                      key={event.id}
                      className={`${failed ? 'failure' : 'ok'}${selectedRow ? ' selected' : ''}`}
                      onClick={() => setSelectedId(selectedRow ? null : event.id)}
                    >
                      <td className="log-col-id mono">{event.id}</td>
                      <td><time dateTime={new Date(event.timestamp_unix_ms).toISOString()}>{formatTime(event.timestamp_unix_ms)}</time></td>
                      <td><span className={`badge ${failed ? 'failed' : 'ready'}`}>{failed ? t('common.failed') : t('common.success')}</span></td>
                      <td className="mono">{event.http_status ?? '—'}</td>
                      <td>{auditModule(event)}</td>
                      <td>{auditAction(event)}</td>
                      <td className="log-path">
                        <span className={`method method-${(event.http_method ?? '').toLowerCase()}`}>{event.http_method ?? '—'}</span>
                        <code title={event.http_path ?? event.message}>{event.http_path ?? event.message}</code>
                      </td>
                      <td>{event.actor ?? t('events.unauthenticated')}</td>
                      <td className="log-col-ip mono">{event.client_ip ?? '—'}</td>
                      <td className="mono">{formatDuration(event.duration_us)}</td>
                      <td className="log-col-error">{clip(eventError(event), 36)}</td>
                      <td className="log-col-trace mono">{clip(event.request_id, 18)}</td>
                    </tr>
                  )
                }
                return (
                  <tr
                    key={event.id}
                    className={`${failed ? 'failure' : event.severity}${selectedRow ? ' selected' : ''}`}
                    onClick={() => setSelectedId(selectedRow ? null : event.id)}
                  >
                    <td className="log-col-id mono">{event.id}</td>
                    <td><time dateTime={new Date(event.timestamp_unix_ms).toISOString()}>{formatTime(event.timestamp_unix_ms)}</time></td>
                    <td><span className={`badge ${event.severity === 'error' ? 'failed' : event.severity === 'warning' ? 'busy' : 'ready'}`}>{severityLabels()[event.severity]}</span></td>
                    <td>{categoryLabels()[event.category] ?? event.category}</td>
                    <td>{event.kind}</td>
                    <td className="log-path">{event.message}</td>
                    <td>{[event.plugin_id, event.lease_id, event.job_id, event.actor].filter(Boolean).join(' · ') || '—'}</td>
                  </tr>
                )
              }) : (
                <tr className="log-empty">
                  <td colSpan={view === 'audit' ? 12 : 7}>
                    <div className="empty-state">
                      <Icon name={view === 'audit' ? 'Braces' : 'Activity'} size={20} />
                      <div>
                        <strong>{loading ? t('events.loading') : t('events.noRecords')}</strong>
                        <span>{loading ? t('events.loadingHint') : t('events.noRecordsHint')}</span>
                      </div>
                    </div>
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>

        {nextCursor && (
          <button className="event-load-more" type="button" onClick={() => void loadMore()} disabled={loadingMore}>
            {loadingMore ? <Icon name="LoaderCircle" size={14} className="spin" /> : null}
            {t('events.loadMore')}
          </button>
        )}
      </section>

      {selected && (
        <section className="section-block log-detail" aria-label={t('events.recordDetails')}>
          <div className="section-heading">
            <div>
              <span className="eyebrow">{t('events.recordDetails')}</span>
              <h2>{selected.http_path ?? selected.message}</h2>
            </div>
            <button type="button" className="text-button primary" onClick={() => setSelectedId(null)}>{t('common.close')}</button>
          </div>
          <dl className="detail-list">
            <div><dt>{t('events.id')}</dt><dd>{selected.id}</dd></div>
            <div><dt>{t('events.globalRequestId')}</dt><dd>{selected.request_id ?? '—'}</dd></div>
            <div><dt>{t('events.recordedAt')}</dt><dd>{formatTime(selected.timestamp_unix_ms)}</dd></div>
            <div><dt>{t('events.outcome')}</dt><dd>{isFailure(selected) ? t('common.failed') : t('common.success')}</dd></div>
            {selected.http_status !== null && <div><dt>{t('events.httpStatusCode')}</dt><dd>{selected.http_status}</dd></div>}
            <div><dt>{t('events.businessModule')}</dt><dd>{auditModule(selected)}</dd></div>
            <div><dt>{t('events.action')}</dt><dd>{auditAction(selected)}</dd></div>
            {selected.http_method && <div><dt>{t('common.api')}</dt><dd>{selected.http_method} {selected.http_path}</dd></div>}
            <div><dt>{t('events.caller')}</dt><dd>{selected.actor ?? t('events.unauthenticated')}</dd></div>
            <div><dt>{t('events.keyRole')}</dt><dd>{roleLabel(metaString(selected, 'key_role'))}</dd></div>
            <div><dt>{t('events.keyFingerprint')}</dt><dd>{metaString(selected, 'key_fingerprint') ?? '—'}</dd></div>
            <div><dt>{t('events.clientIp')}</dt><dd>{selected.client_ip ?? '—'}</dd></div>
            <div><dt>{t('events.peerIp')}</dt><dd>{selected.peer_ip ?? '—'}</dd></div>
            <div><dt>{t('events.elapsed')}</dt><dd>{formatDuration(selected.duration_us)}</dd></div>
            <div><dt>{t('events.requestSize')}</dt><dd>{formatBytes(selected.request_bytes)}</dd></div>
            <div><dt>{t('events.responseSize')}</dt><dd>{formatBytes(selected.response_bytes)}</dd></div>
            <div><dt>{t('events.requestType')}</dt><dd>{metaString(selected, 'request_content_type') ?? '—'}</dd></div>
            <div><dt>{t('events.responseType')}</dt><dd>{metaString(selected, 'response_content_type') ?? '—'}</dd></div>
            <div><dt>{t('events.queryKeys')}</dt><dd>{metaList(selected, 'query_keys').join(', ') || t('common.none')}</dd></div>
            <div><dt>{t('events.requiredScope')}</dt><dd>{metaString(selected, 'required_scope') ?? '—'}</dd></div>
            <div><dt>{t('events.grantedScopes')}</dt><dd>{metaList(selected, 'granted_scopes').join(', ') || '—'}</dd></div>
            {selected.plugin_id && <div><dt>{t('events.plugin')}</dt><dd>{selected.plugin_id}</dd></div>}
            {selected.job_id && <div><dt>{t('events.job')}</dt><dd>{selected.job_id}</dd></div>}
            {selected.lease_id && <div><dt>{t('events.lease')}</dt><dd>{selected.lease_id}</dd></div>}
            <div><dt>User-Agent</dt><dd>{metaString(selected, 'user_agent') ?? '—'}</dd></div>
            {eventError(selected) && <div><dt>{t('events.errorMessage')}</dt><dd className="job-error">{eventError(selected)}</dd></div>}
          </dl>
          {selected.metadata && Object.keys(selected.metadata).length > 0 && (
            <pre>{JSON.stringify(selected.metadata, null, 2)}</pre>
          )}
        </section>
      )}
    </>
  )
}
