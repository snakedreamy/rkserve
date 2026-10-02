import { t, number, UiError, errorText } from '../i18n/core'
import { useI18n } from '../i18n/useI18n'
import { Icon } from '../icons/Icon'
import { useEffect, useRef, useState, type RefObject } from 'react'
import { Link, Navigate, useParams } from 'react-router-dom'
import { api } from '../api'
import { useSystem } from '../app/SystemContext'
import { ConfigField } from '../components/ConfigField'
import { PageHeading } from '../components/PageHeading'
import type {
  CapabilityDefinition,
  CapabilityRenderer,
  InferenceJob,
  JobResult,
  PluginSummary,
} from '../types'
import { AudioInputStage, AudioOutput } from './renderers/AudioRenderer'
import {
  DetectionOverlay,
  DetectionOutput,
  parseDetectionResult,
} from './renderers/DetectionRenderer'
import { PoseOverlay, PoseOutput, parsePoseResult } from './renderers/PoseRenderer'
import {
  SpeechTranscriptionOutput,
  parseSpeechTranscription,
} from './renderers/SpeechRenderer'

export function PluginRunPage() {
  useI18n()
  const { pluginId = '' } = useParams()
  const { plugins, workers, loading, runAction, actionId } = useSystem()
  const plugin = plugins.find((item) => item.id === pluginId)
  const [capabilityId, setCapabilityId] = useState('')
  const [file, setFile] = useState<File | null>(null)
  const [textInput, setTextInput] = useState('')
  const [parameters, setParameters] = useState<Record<string, string>>({})
  const [inputUrl, setInputUrl] = useState<string | null>(null)
  const [outputUrl, setOutputUrl] = useState<string | null>(null)
  const [imageSize, setImageSize] = useState<{ width: number; height: number } | null>(null)
  const audioRef = useRef<HTMLAudioElement>(null)
  const capability =
    plugin?.capabilities.find((item) => item.id === capabilityId) ?? plugin?.capabilities[0]
  const { job, setJob, result, setResult, jobError, setJobError } = useJobPoller(
    pluginId,
    capability,
  )
  const ready = workers.some(
    (worker) => worker.plugin_id === pluginId && worker.status === 'ready',
  )
  const jobActive = isJobActive(job)

  useEffect(() => {
    if (
      plugin?.capabilities.length &&
      !plugin.capabilities.some((item) => item.id === capabilityId)
    ) {
      setCapabilityId(plugin.capabilities[0].id)
    }
  }, [plugin?.id, capabilityId])

  useEffect(() => {
    if (!file || !['image', 'audio'].includes(capability?.input_kind ?? '')) {
      setInputUrl(null)
      return
    }
    const url = URL.createObjectURL(file)
    setInputUrl(url)
    return () => URL.revokeObjectURL(url)
  }, [file, capability?.input_kind])

  useEffect(() => {
    if (!result?.blob) {
      setOutputUrl(null)
      return
    }
    const url = URL.createObjectURL(result.blob)
    setOutputUrl(url)
    return () => URL.revokeObjectURL(url)
  }, [result?.blob])

  useEffect(() => {
    setFile(null)
    setResult(null)
    setJob(null)
    setJobError(null)
    setTextInput('')
    setImageSize(null)
    setParameters(
      Object.fromEntries(
        (capability?.parameters ?? []).map((parameter) => [
          parameter.key,
          parameter.default ?? '',
        ]),
      ),
    )
  }, [capability?.id, setJob, setJobError, setResult])

  if (!plugin && loading) return null
  if (!plugin) return <Navigate to="/plugins" replace />

  async function submitJob() {
    if (!capability) return
    const body = capability.input_kind === 'text' ? textInput : file
    if (!body) return
    setResult(null)
    setJobError(null)
    setJob(null)
    const submitted = await runAction(`job:${capability.id}`, () =>
      api.submitJob(plugin!.id, capability, body, parameters),
    )
    if (submitted) setJob(submitted)
  }

  async function cancelCurrentJob() {
    if (!job) return
    const canceled = await runAction(`cancel:${job.id}`, () =>
      api.cancelJob(plugin!.id, job.id),
    )
    if (canceled) setJob(canceled)
  }

  function selectFile(next: File | null) {
    setFile(next)
    setResult(null)
    setJob(null)
    setJobError(null)
    setImageSize(null)
  }

  function replayAudio() {
    const audio = audioRef.current
    if (!audio) return
    audio.currentTime = 0
    void audio
      .play()
      .catch((error: unknown) =>
        setJobError(error instanceof Error ? error : new UiError('error.audioPlayback')),
      )
  }

  const downloadName = job
    ? `${plugin.id}-${job.id}${resultExtension(result?.contentType)}`
    : undefined

  return (
    <>
      <Link className="back-link" to={`/plugins/${plugin.id}`}>
        <Icon name="ArrowLeft" size={14} />{t('run.back')}
      </Link>
      <PageHeading
        eyebrow={t('run.eyebrow')}
        title={plugin.name}
        description={t('run.description', { plugin: plugin.id })}
      />
      {!ready ? (
        <section className="test-unavailable">
          <strong>{t('run.notReady')}</strong>
          <span>{t('run.notReadyHint')}</span>
          <Link to="/npu">{t('run.goCompute')}</Link>
        </section>
      ) : !capability ? (
        <section className="test-unavailable">
          <strong>{t('run.noCapabilities')}</strong>
          <span>{t('run.checkManifest')}</span>
        </section>
      ) : (
        <>
          <CapabilitySelector
            plugin={plugin}
            capability={capability}
            disabled={jobActive}
            onChange={setCapabilityId}
          />
          <div className="test-workspace">
            <JobInput
              capability={capability}
              file={file}
              textInput={textInput}
              parameters={parameters}
              inputUrl={inputUrl}
              imageSize={imageSize}
              result={result}
              jobActive={jobActive}
              actionPending={actionId === `job:${capability.id}`}
              onFileChange={selectFile}
              onTextChange={setTextInput}
              onParameterChange={(key, value) =>
                setParameters((current) => ({ ...current, [key]: value }))
              }
              onImageSizeChange={setImageSize}
              onSubmit={() => void submitJob()}
            />
            <JobOutput
              capability={capability}
              job={job}
              jobError={jobError}
              result={result}
              outputUrl={outputUrl}
              downloadName={downloadName}
              cancelPending={Boolean(job && actionId === `cancel:${job.id}`)}
              audioRef={audioRef}
              onCancel={() => void cancelCurrentJob()}
              onReplayAudio={replayAudio}
            />
          </div>
        </>
      )}
    </>
  )
}

function useJobPoller(pluginId: string, capability: CapabilityDefinition | undefined) {
  const [result, setResult] = useState<JobResult | null>(null)
  const [job, setJob] = useState<InferenceJob | null>(null)
  const [jobError, setJobError] = useState<unknown>(null)

  useEffect(() => {
    if (!job?.id || !capability) return
    const jobId = job.id
    const selectedCapability = capability
    let disposed = false
    let timer = 0

    function schedule() {
      timer = window.setTimeout(() => void poll(), 800)
    }

    async function poll() {
      try {
        const next = await api.job(pluginId, jobId)
        if (disposed) return
        setJob(next)
        setJobError(null)
        if (isJobActive(next)) {
          schedule()
          return
        }
        if (next.state === 'succeeded') {
          const response = await api.jobResult(pluginId, next.id, selectedCapability)
          if (!disposed) setResult(response)
        }
      } catch (error: unknown) {
        if (disposed) return
        setJobError(error instanceof Error ? error : new UiError('error.operationFailed'))
        schedule()
      }
    }

    schedule()
    return () => {
      disposed = true
      window.clearTimeout(timer)
    }
  }, [job?.id, capability?.id, pluginId])

  return { job, setJob, result, setResult, jobError, setJobError }
}

// ── CapabilitySelector ────────────────────────────────────────────────────────

interface CapabilitySelectorProps {
  plugin: PluginSummary
  capability: CapabilityDefinition
  disabled: boolean
  onChange: (capabilityId: string) => void
}

function CapabilitySelector({
  plugin,
  capability,
  disabled,
  onChange,
}: CapabilitySelectorProps) {
  useI18n()
  return (
    <section className="capability-selector">
      <div>
        <span className="eyebrow">{t('run.capability')}</span>
        <strong>{capability.name}</strong>
        <span>{capability.description}</span>
      </div>
      <div className="selector-right">
        {plugin.capabilities.length > 1 && (
          <div className="seg-group" role="tablist" aria-label={t('run.switchCapability')}>
            {plugin.capabilities.map((item) => (
              <button
                type="button"
                role="tab"
                aria-selected={item.id === capability.id}
                key={item.id}
                className={`seg-item ${item.id === capability.id ? 'active' : ''}`}
                disabled={disabled}
                onClick={() => onChange(item.id)}
              >
                {item.name}
              </button>
            ))}
          </div>
        )}
        <code>{capability.id}</code>
      </div>
    </section>
  )
}

// ── JobInput ──────────────────────────────────────────────────────────────────

interface JobInputProps {
  capability: CapabilityDefinition
  file: File | null
  textInput: string
  parameters: Record<string, string>
  inputUrl: string | null
  imageSize: { width: number; height: number } | null
  result: JobResult | null
  jobActive: boolean
  actionPending: boolean
  onFileChange: (file: File | null) => void
  onTextChange: (value: string) => void
  onParameterChange: (key: string, value: string) => void
  onImageSizeChange: (size: { width: number; height: number }) => void
  onSubmit: () => void
}

function JobInput({
  capability,
  file,
  textInput,
  parameters,
  inputUrl,
  imageSize,
  result,
  jobActive,
  actionPending,
  onFileChange,
  onTextChange,
  onParameterChange,
  onImageSizeChange,
  onSubmit,
}: JobInputProps) {
  useI18n()
  const accept = capability.accepted_content_types.join(',')
  const renderer = resolveRenderer(capability)
  const detection =
    capability.output_kind === 'detections' ? parseDetectionResult(result?.data) : null
  const pose = renderer === 'image_pose_overlay' ? parsePoseResult(result?.data) : null
  const textBytes = new TextEncoder().encode(textInput).byteLength
  const textTooLarge = capability.input_kind === 'text' && textBytes > capability.max_input_bytes
  const fileTooLarge = Boolean(file && file.size > capability.max_input_bytes)
  const inputTooLarge = textTooLarge || fileTooLarge

  return (
    <section className="test-input">
      <div className="section-heading">
        <div>
          <span className="eyebrow">{t('run.inputKind', { kind: t(`kind.${capability.input_kind}`) })}</span>
          <h2>{capability.input_kind === 'text' ? t('run.requestText') : t('run.inputFile')}</h2>
        </div>
        {capability.input_kind !== 'text' && (
          <label className={`upload-button ${jobActive ? 'disabled' : ''}`}>
            <Icon name="FileUp" size={15} />{t('run.chooseFile')}
            <input
              type="file"
              accept={accept}
              disabled={jobActive}
              onChange={(event) => onFileChange(event.target.files?.[0] ?? null)}
            />
          </label>
        )}
      </div>
      {capability.presentation?.input_hint && (
        <p className="input-hint">{capability.presentation.input_hint}</p>
      )}
      {capability.input_kind === 'text' ? (
        <>
          {(capability.presentation?.text_examples.length ?? 0) > 0 && (
            <div className="text-examples">
              <span>{t('run.examples')}</span>
              {capability.presentation!.text_examples.map((example, index) => (
                <button
                  type="button"
                  key={`${index}-${example}`}
                  disabled={jobActive}
                  title={example}
                  onClick={() => onTextChange(example)}
                >
                  {example}
                </button>
              ))}
            </div>
          )}
          <textarea
            className="text-input"
            aria-label={t('run.requestText')}
            value={textInput}
            disabled={jobActive}
            onChange={(event) => onTextChange(event.target.value)}
            placeholder={
              capability.presentation?.input_placeholder ??
              t('run.inputPlaceholder', { type: capability.accepted_content_types[0] })
            }
          />
          <div className={`text-input-meta ${inputTooLarge ? 'invalid' : ''}`}>
            <span>{t('run.utf8')}</span>
            <strong>
              {number(textBytes)} / {number(capability.max_input_bytes)} {t('common.bytes')}
            </strong>
          </div>
        </>
      ) : (
        <>
          {capability.input_kind === 'audio' ? (
            <AudioInputStage
              acceptedContentTypes={capability.accepted_content_types}
              file={file}
              inputUrl={inputUrl}
            />
          ) : (
            <ImageStage
              capability={capability}
              file={file}
              inputUrl={inputUrl}
              imageSize={imageSize}
              renderer={renderer}
              detection={detection}
              pose={pose}
              onImageSizeChange={onImageSizeChange}
            />
          )}
          {file && (
            <div className={`file-input-meta ${fileTooLarge ? 'invalid' : ''}`}>
              <span>{file.name}</span>
              <strong>
                {number(file.size)} / {number(capability.max_input_bytes)} {t('common.bytes')}
              </strong>
            </div>
          )}
        </>
      )}
      {capability.parameters.length > 0 && (
        <div className="test-parameters">
          <span className="eyebrow">{t('run.parameters')}</span>
          {capability.parameters.map((parameter) => (
            <label key={parameter.key}>
              <span>
                <strong>{parameter.label}</strong>
                <small>{parameter.description}</small>
              </span>
              <ConfigField
                field={parameter}
                value={parameters[parameter.key] ?? parameter.default ?? ''}
                emptyLabel={t('run.default')}
                numberStep="0.05"
                onChange={(value) => onParameterChange(parameter.key, value)}
              />
            </label>
          ))}
        </div>
      )}
      <button
        className="invoke-button"
        type="button"
        disabled={
          inputTooLarge ||
          jobActive ||
          actionPending ||
          (capability.input_kind === 'text' ? !textInput : !file)
        }
        onClick={onSubmit}
      >
        {actionPending ? <Icon name="LoaderCircle" size={15} className="spin" /> : <Icon name="Play" size={15} />}
        {t('run.submit')}
      </button>
    </section>
  )
}

// ── ImageStage ────────────────────────────────────────────────────────────────

interface ImageStageProps {
  capability: CapabilityDefinition
  file: File | null
  inputUrl: string | null
  imageSize: { width: number; height: number } | null
  renderer: CapabilityRenderer | null
  detection: ReturnType<typeof parseDetectionResult>
  pose: ReturnType<typeof parsePoseResult>
  onImageSizeChange: (size: { width: number; height: number }) => void
}

function ImageStage({
  capability,
  file,
  inputUrl,
  imageSize,
  renderer,
  detection,
  pose,
  onImageSizeChange,
}: ImageStageProps) {
  useI18n()
  const accept = capability.accepted_content_types.join(',')
  return (
    <div className="image-stage">
      {inputUrl ? (
        <div className="image-canvas">
          <img
            src={inputUrl}
            alt={t('run.preview')}
            onLoad={(event) =>
              onImageSizeChange({
                width: event.currentTarget.naturalWidth,
                height: event.currentTarget.naturalHeight,
              })
            }
          />
          {renderer === 'image_detection_overlay' && imageSize && detection && (
            <DetectionOverlay imageSize={imageSize} detection={detection} />
          )}
          {renderer === 'image_pose_overlay' && imageSize && pose && (
            <PoseOverlay imageSize={imageSize} pose={pose} />
          )}
        </div>
      ) : (
        <div className="image-placeholder">
          <Icon name="ScanSearch" size={26} />
          <span>{file ? file.name : t('run.supports', { types: accept })}</span>
          <small>{t('run.fileLimit', { size: number(capability.max_input_bytes / 1024 / 1024, 1) })}</small>
        </div>
      )}
    </div>
  )
}

// ── JobOutput ─────────────────────────────────────────────────────────────────

interface JobOutputProps {
  capability: CapabilityDefinition
  job: InferenceJob | null
  jobError: unknown
  result: JobResult | null
  outputUrl: string | null
  downloadName: string | undefined
  cancelPending: boolean
  audioRef: RefObject<HTMLAudioElement | null>
  onCancel: () => void
  onReplayAudio: () => void
}

function JobOutput({
  capability,
  job,
  jobError,
  result,
  outputUrl,
  downloadName,
  cancelPending,
  audioRef,
  onCancel,
  onReplayAudio,
}: JobOutputProps) {
  useI18n()
  const renderer = resolveRenderer(capability)
  const detection =
    capability.output_kind === 'detections' ? parseDetectionResult(result?.data) : null
  const pose = renderer === 'image_pose_overlay' ? parsePoseResult(result?.data) : null
  const transcription = renderer === 'speech_transcription'
    ? parseSpeechTranscription(result?.data)
    : null
  const totalUs = result
    ? Object.values(result.timings).reduce((total, value) => total + value, 0)
    : 0
  const jobActive = isJobActive(job)

  return (
    <aside className="test-output">
      <span className="eyebrow">{t('run.outputKind', { kind: t(`kind.${capability.output_kind}`) })}</span>
      <h2>{t('run.result')}</h2>
      {job && (
        <JobStatus
          job={job}
          error={job.error ?? jobError}
          active={jobActive}
          cancelPending={cancelPending}
          onCancel={onCancel}
        />
      )}
      {result ? (
        <>
          <div className="inference-timing">
            <strong>{number(result.timings.inferenceUs / 1000, 1)}</strong>
            <span>{t('run.timing', { total: number(totalUs / 1000, 1) })}</span>
          </div>
          {renderer === 'image_detection_overlay' ? (
            detection ? (
              <DetectionOutput
                detection={detection}
                outputUrl={outputUrl}
                downloadName={downloadName}
                rawText={result.rawText}
              />
            ) : (
              <p className="job-error">{t('run.invalidDetection')}</p>
            )
          ) : renderer === 'image_pose_overlay' ? (
            pose ? (
              <PoseOutput
                pose={pose}
                outputUrl={outputUrl}
                downloadName={downloadName}
                rawText={result.rawText}
              />
            ) : (
              <p className="job-error">{t('run.invalidPose')}</p>
            )
          ) : renderer === 'speech_transcription' ? (
            transcription ? (
              <SpeechTranscriptionOutput
                transcription={transcription}
                outputUrl={outputUrl}
                downloadName={downloadName}
              />
            ) : (
              <p className="job-error">{t('run.invalidTranscription')}</p>
            )
          ) : renderer === 'audio_player' && outputUrl ? (
            <AudioOutput
              result={result}
              outputUrl={outputUrl}
              downloadName={downloadName}
              audioRef={audioRef}
              onReplay={onReplayAudio}
            />
          ) : capability.output_kind === 'text' && typeof result.data === 'string' ? (
            <TranscriptionOutput
              text={result.data}
              outputUrl={outputUrl}
              downloadName={downloadName}
            />
          ) : capability.output_kind === 'binary' ? (
            <BinaryOutput result={result} outputUrl={outputUrl} downloadName={downloadName} />
          ) : (
            <pre className="generic-output">
              {typeof result.data === 'string'
                ? result.data
                : JSON.stringify(result.data, null, 2)}
            </pre>
          )}
        </>
      ) : (
        !job && <p className="muted-copy">{t('run.emptyResult')}</p>
      )}
    </aside>
  )
}

// ── JobStatus ─────────────────────────────────────────────────────────────────

interface JobStatusProps {
  job: InferenceJob
  error: unknown
  active: boolean
  cancelPending: boolean
  onCancel: () => void
}

function JobStatus({ job, error, active, cancelPending, onCancel }: JobStatusProps) {
  useI18n()
  const labels = {
    queued: t('job.queued'),
    running: t('job.running'),
    canceling: t('job.canceling'),
    succeeded: t('job.succeeded'),
    failed: t('common.failed'),
    canceled: t('job.canceled'),
  }
  const icon =
    job.state === 'queued' ? (
      <Icon name="Clock3" size={15} />
    ) : job.state === 'succeeded' ? (
      <Icon name="CheckCircle2" size={15} />
    ) : job.state === 'canceled' || job.state === 'failed' ? (
      <Icon name="CircleStop" size={15} />
    ) : (
      <Icon name="LoaderCircle" size={15} className="spin" />
    )

  return (
    <div className={`job-status ${job.state}`}>
      <div className="job-status-heading">
        <span>
          {icon}
          <strong>{labels[job.state]}</strong>
        </span>
        <code>{job.id}</code>
      </div>
      <div className="job-track">
        <i className={job.state !== 'queued' ? 'done' : 'active'} />
        <i
          className={
            job.state === 'running' || job.state === 'canceling'
              ? 'active'
              : ['succeeded', 'failed', 'canceled'].includes(job.state)
                ? 'done'
                : ''
          }
        />
        <i
          className={
            job.state === 'succeeded'
              ? 'done'
              : job.state === 'failed' || job.state === 'canceled'
                ? 'stopped'
                : ''
          }
        />
      </div>
      <div className="job-meta">
        <span>
          {job.started_at_unix_ms
            ? t('job.elapsed', { seconds: number(((job.finished_at_unix_ms ?? Date.now()) - job.started_at_unix_ms) / 1000, 1) })
            : t('job.waitingWorker')}
        </span>
        <span>
          {job.result_bytes
            ? t('job.resultBytes', { size: number(job.result_bytes) })
            : t('job.resultStore')}
        </span>
      </div>
      {active && (
        <button
          className="job-cancel"
          type="button"
          disabled={job.state === 'canceling' || cancelPending}
          onClick={onCancel}
        >
          <Icon name="CircleStop" size={13} />
          {job.state === 'canceling' ? t('job.waitingCancel') : t('job.cancel')}
        </button>
      )}
      {Boolean(error) && <p className="job-error">{errorText(error)}</p>}
    </div>
  )
}

// ── Text / Binary fallbacks ───────────────────────────────────────────────────

function TranscriptionOutput({
  text,
  outputUrl,
  downloadName,
}: {
  text: string
  outputUrl: string | null
  downloadName: string | undefined
}) {
  useI18n()
  const [copied, setCopied] = useState(false)

  async function copyText() {
    await navigator.clipboard.writeText(text)
    setCopied(true)
    window.setTimeout(() => setCopied(false), 1400)
  }

  return (
    <div className="transcription-result">
      <span className="eyebrow">{t('result.transcription')}</span>
      <p>{text || t('result.noSpeech')}</p>
      <div className="result-actions">
        <button type="button" onClick={() => void copyText()}>
          <Icon name={copied ? 'Check' : 'Copy'} size={14} />
          {copied ? t('common.copied') : t('common.copyText')}
        </button>
        {outputUrl && (
          <a href={outputUrl} download={downloadName}>
            <Icon name="Download" size={14} />{t('common.downloadText')}
          </a>
        )}
      </div>
    </div>
  )
}

function BinaryOutput({
  result,
  outputUrl,
  downloadName,
}: {
  result: JobResult
  outputUrl: string | null
  downloadName: string | undefined
}) {
  useI18n()
  return (
    <div>
      <p className="muted-copy">
        {t('result.received', { size: number(result.blob.size), type: result.contentType })}
      </p>
      {outputUrl && (
        <div className="result-actions">
          <a href={outputUrl} download={downloadName}>
            <Icon name="Download" size={14} />{t('common.downloadResult')}
          </a>
        </div>
      )}
    </div>
  )
}

// ── Utilities ─────────────────────────────────────────────────────────────────

function isJobActive(job: InferenceJob | null) {
  return Boolean(job && ['queued', 'running', 'canceling'].includes(job.state))
}

function resolveRenderer(capability: CapabilityDefinition): CapabilityRenderer | null {
  if (capability.presentation?.renderer) return capability.presentation.renderer
  if (capability.output_kind === 'audio') return 'audio_player'
  if (capability.input_kind === 'image' && capability.output_kind === 'detections') {
    return 'image_detection_overlay'
  }
  return null
}

function resultExtension(contentType: string | undefined) {
  if (contentType === 'audio/wav' || contentType === 'audio/x-wav') return '.wav'
  if (contentType === 'application/json') return '.json'
  if (contentType?.startsWith('text/')) return '.txt'
  return '.bin'
}
