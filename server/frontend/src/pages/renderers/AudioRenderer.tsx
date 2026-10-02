import { t, number } from '../../i18n/core'
import { useI18n } from '../../i18n/useI18n'
import { Icon } from '../../icons/Icon'
import { useState } from 'react'
import type { RefObject } from 'react'
import type { JobResult } from '../../types'

// ── Audio Input Stage ────────────────────────────────────────────────────────

export function AudioInputStage({
  acceptedContentTypes,
  file,
  inputUrl,
}: {
  acceptedContentTypes: string[]
  file: File | null
  inputUrl: string | null
}) {
  useI18n()
  return (
    <div className="audio-input-stage">
      <div className="audio-input-heading">
        <Icon name="AudioLines" size={22} />
        <span>
          <strong>{file?.name ?? t('audio.waiting')}</strong>
          <small>
            {file
              ? `${number(file.size / 1024, 1)} KiB · ${file.type || 'audio/wav'}`
              : acceptedContentTypes.join(' · ')}
          </small>
        </span>
      </div>
      {inputUrl ? (
        <audio aria-label={t('audio.inputPreview')} className="audio-input-preview" controls preload="metadata" src={inputUrl} />
      ) : (
        <p>{t('audio.previewHint')}</p>
      )}
    </div>
  )
}

// ── Audio Output ─────────────────────────────────────────────────────────────

export function AudioOutput({
  result,
  outputUrl,
  downloadName,
  audioRef,
  onReplay,
}: {
  result: JobResult
  outputUrl: string
  downloadName: string | undefined
  audioRef: RefObject<HTMLAudioElement | null>
  onReplay: () => void
}) {
  useI18n()
  return (
    <div className="audio-result">
      <div className="result-summary">
        <strong>{t('audio.generated')}</strong>
        <span>
          {number(result.blob.size)} {t('common.bytes')} · {result.contentType}
        </span>
      </div>
      <audio aria-label={t('audio.output')} ref={audioRef} className="audio-output" controls preload="metadata" src={outputUrl} />
      <div className="result-actions">
        <button type="button" onClick={onReplay}>
          <Icon name="RotateCcw" size={14} />{t('audio.replay')}
        </button>
        <a href={outputUrl} download={downloadName}>
          <Icon name="Download" size={14} />{t('audio.download')}
        </a>
      </div>
    </div>
  )
}
