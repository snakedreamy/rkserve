import type { Allocation, ApiPrincipal, CapabilityDefinition, CoreMask, DeviceTelemetry, EventFilters, EventPage, InferenceJob, JobResult, NpuTopology, PluginSpec, PluginSummary, WorkerSnapshot } from './types'

import { UiError } from './i18n/core'

const API_KEY_STORAGE = 'rkserve.apiKey'
export const AUTH_REQUIRED_EVENT = 'rkserve:auth-required'

export class ApiError extends UiError {
  constructor(
    private readonly serverMessage: string | null,
    public readonly status: number,
    public readonly requestId: string | null,
    public readonly kind: string | null = null,
  ) {
    super('error.requestFailed', { status })
    if (serverMessage !== null) this.message = serverMessage
    this.name = 'ApiError'
  }

  override localizedMessage(): string {
    return this.serverMessage ?? super.localizedMessage()
  }
}

export function getApiKey() {
  return window.sessionStorage.getItem(API_KEY_STORAGE)
}

export function setApiKey(value: string) {
  window.sessionStorage.setItem(API_KEY_STORAGE, value)
}

export function clearApiKey() {
  window.sessionStorage.removeItem(API_KEY_STORAGE)
}

async function responseError(response: Response) {
  const body = await response.json().catch(() => null)
  const error = new ApiError(
    typeof body?.error === 'string' ? body.error : null,
    response.status,
    response.headers.get('x-request-id'),
    typeof body?.kind === 'string' ? body.kind : null,
  )
  if (response.status === 401 || error.kind === 'role_denied') {
    window.dispatchEvent(new CustomEvent(AUTH_REQUIRED_EVENT))
  }
  return error
}

export function authorizedFetch(path: string, init?: RequestInit) {
  const headers = new Headers(init?.headers)
  const key = getApiKey()
  if (key) headers.set('Authorization', `Bearer ${key}`)
  return fetch(path, { ...init, headers })
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  // JSON is the control-plane default. Binary Job uploads bypass this helper
  // and set the capability's declared Content-Type explicitly.
  const headers = new Headers(init?.headers)
  headers.set('Content-Type', 'application/json')
  const response = await authorizedFetch(path, { ...init, headers })
  if (!response.ok) {
    throw await responseError(response)
  }
  if (response.status === 204) return undefined as T
  return response.json() as Promise<T>
}

async function jobResultResponse(response: Response, capability: CapabilityDefinition): Promise<JobResult> {
  if (!response.ok) {
    throw await responseError(response)
  }
  const contentType = response.headers.get('content-type')?.split(';')[0] ?? capability.output_content_type
  const blob = await response.blob()
  const rawText = capability.output_kind === 'audio' || capability.output_kind === 'binary' ? null : await blob.text()
  const data = rawText === null ? blob : contentType === 'application/json' ? JSON.parse(rawText) : rawText
  return {
    contentType,
    data,
    blob,
    rawText,
    timings: {
      queueUs: Number(response.headers.get('x-rkserve-queue-us') ?? 0),
      preprocessUs: Number(response.headers.get('x-rkserve-preprocess-us') ?? 0),
      inferenceUs: Number(response.headers.get('x-rkserve-inference-us') ?? 0),
      postprocessUs: Number(response.headers.get('x-rkserve-postprocess-us') ?? 0),
    },
  }
}

function pluginPath(pluginId: string) {
  return `/api/v1/plugins/${encodeURIComponent(pluginId)}`
}

export const api = {
  whoami: () => request<ApiPrincipal>('/api/v1/auth/whoami'),
  topology: () => request<NpuTopology>('/api/v1/npu/topology'),
  telemetry: () => request<DeviceTelemetry>('/api/v1/system/telemetry'),
  allocations: () => request<Allocation[]>('/api/v1/scheduler/allocations'),
  plugins: () => request<PluginSummary[]>('/api/v1/plugins'),
  pluginSpec: (pluginId: string) => request<PluginSpec>(`${pluginPath(pluginId)}/spec`),
  workers: () => request<WorkerSnapshot[]>('/api/v1/workers'),
  events: (filters: EventFilters = {}) => {
    const query = new URLSearchParams()
    const values: Record<string, string | number | undefined> = {
      after: filters.after,
      cursor: filters.cursor,
      severity: filters.severity,
      category: filters.category,
      kind: filters.kind,
      plugin_id: filters.pluginId,
      lease_id: filters.leaseId,
      job_id: filters.jobId,
      request_id: filters.requestId,
      actor: filters.actor,
      client_ip: filters.clientIp,
      http_method: filters.httpMethod,
      http_status: filters.httpStatus,
      source: filters.source,
      exclude_source: filters.excludeSource,
      q: filters.q,
      outcome: filters.outcome,
      path_prefix: filters.pathPrefix,
      from: filters.from,
      to: filters.to,
      limit: filters.limit ?? 100,
    }
    for (const [key, value] of Object.entries(values)) {
      if (value !== undefined && value !== '') query.set(key, String(value))
    }
    return request<EventPage>(`/api/v1/events?${query}`)
  },
  allocate: (pluginId: string, coreMask: CoreMask) =>
    request<Allocation>('/api/v1/scheduler/allocations', {
      method: 'POST',
      body: JSON.stringify({ plugin_id: pluginId, core_mask: coreMask }),
    }),
  release: (leaseId: string) =>
    request<void>(`/api/v1/scheduler/allocations/${encodeURIComponent(leaseId)}`, { method: 'DELETE' }),
  configure: (pluginId: string, values: Record<string, string>) =>
    request<Record<string, string>>(`${pluginPath(pluginId)}/configuration`, {
      method: 'PUT',
      body: JSON.stringify({ values }),
    }),
  submitJob: async (pluginId: string, capability: CapabilityDefinition, body: Blob | string, parameters: Record<string, string> = {}): Promise<InferenceJob> => {
    const contentType = body instanceof Blob
      ? body.type || capability.accepted_content_types[0]
      : capability.accepted_content_types[0]
    const query = new URLSearchParams(Object.entries(parameters).filter(([, value]) => value !== ''))
    const suffix = query.size ? `?${query}` : ''
    const response = await authorizedFetch(`${pluginPath(pluginId)}/jobs/${encodeURIComponent(capability.id)}${suffix}`, {
      method: 'POST',
      headers: { 'Content-Type': contentType },
      body,
    })
    if (!response.ok) {
      throw await responseError(response)
    }
    return response.json() as Promise<InferenceJob>
  },
  job: (pluginId: string, jobId: string) =>
    request<InferenceJob>(`${pluginPath(pluginId)}/jobs/${encodeURIComponent(jobId)}`),
  cancelJob: (pluginId: string, jobId: string) =>
    request<InferenceJob>(`${pluginPath(pluginId)}/jobs/${encodeURIComponent(jobId)}`, { method: 'DELETE' }),
  jobResult: async (pluginId: string, jobId: string, capability: CapabilityDefinition) =>
    jobResultResponse(await authorizedFetch(`${pluginPath(pluginId)}/jobs/${encodeURIComponent(jobId)}/result`), capability),
}
