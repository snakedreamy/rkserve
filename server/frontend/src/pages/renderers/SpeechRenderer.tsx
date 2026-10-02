import { t, number } from '../../i18n/core'
import { useI18n } from '../../i18n/useI18n'
import { Icon } from '../../icons/Icon'
import { useState } from 'react'

// ── Speech transcription types ───────────────────────────────────────────────

export interface SpeechTranscriptionResult {
  text: string
  language: string | null
  emotion: string | null
  events: string[]
  audio_duration_ms: number
  segments: Array<{
    index: number
    start_ms: number
    end_ms: number
    text: string
    language: string | null
    emotion: string | null
    events: string[]
  }>
}

// ── Result panel ─────────────────────────────────────────────────────────────

export function SpeechTranscriptionOutput({
  transcription,
  outputUrl,
  downloadName,
}: {
  transcription: SpeechTranscriptionResult
  outputUrl: string | null
  downloadName: string | undefined
}) {
  useI18n()
  const [copied, setCopied] = useState(false)

  async function copyText() {
    await navigator.clipboard.writeText(transcription.text)
    setCopied(true)
    window.setTimeout(() => setCopied(false), 1400)
  }

  return (
    <div className="speech-transcription-result">
      <div className="transcription-facts">
        <span><small>{t('language.label')}</small><strong>{languageLabel(transcription.language)}</strong></span>
        <span><small>{t('speech.emotion')}</small><strong>{emotionLabel(transcription.emotion)}</strong></span>
        <span><small>{t('speech.duration')}</small><strong>{formatTimestamp(transcription.audio_duration_ms)}</strong></span>
      </div>
      {transcription.events.length > 0 && (
        <div className="transcription-events">
          {transcription.events.map((event) => <span key={event}>{eventLabel(event)}</span>)}
        </div>
      )}
      <div className="transcription-result">
        <span className="eyebrow">{t('result.transcription')}</span>
        <p>{transcription.text || t('result.noSpeech')}</p>
      </div>
      {transcription.segments.length > 1 && (
        <div className="transcription-segments">
          <span className="eyebrow">{t('speech.segments', { count: number(transcription.segments.length) })}</span>
          {transcription.segments.map((segment) => (
            <div key={segment.index}>
              <time>{formatTimestamp(segment.start_ms)} — {formatTimestamp(segment.end_ms)}</time>
              <p>{segment.text || t('speech.noText')}</p>
            </div>
          ))}
        </div>
      )}
      <div className="result-actions">
        <button type="button" onClick={() => void copyText()}>
          <Icon name={copied ? 'Check' : 'Copy'} size={14} />
          {copied ? t('common.copied') : t('common.copyText')}
        </button>
        {outputUrl && (
          <a href={outputUrl} download={downloadName}>
            <Icon name="Download" size={14} />{t('common.downloadJson')}
          </a>
        )}
      </div>
    </div>
  )
}

// ── Helpers ──────────────────────────────────────────────────────────────────

export function parseSpeechTranscription(value: unknown): SpeechTranscriptionResult | null {
  if (!value || typeof value !== 'object') return null
  const item = value as Record<string, unknown>
  if (
    typeof item.text !== 'string' ||
    !(typeof item.language === 'string' || item.language === null) ||
    !(typeof item.emotion === 'string' || item.emotion === null) ||
    !Array.isArray(item.events) ||
    !item.events.every((event) => typeof event === 'string') ||
    !isFiniteNumber(item.audio_duration_ms) ||
    !Array.isArray(item.segments)
  ) return null
  const segments = item.segments.flatMap((value) => {
    if (!value || typeof value !== 'object') return []
    const segment = value as Record<string, unknown>
    if (
      !isFiniteNumber(segment.index) ||
      !isFiniteNumber(segment.start_ms) ||
      !isFiniteNumber(segment.end_ms) ||
      typeof segment.text !== 'string' ||
      !(typeof segment.language === 'string' || segment.language === null) ||
      !(typeof segment.emotion === 'string' || segment.emotion === null) ||
      !Array.isArray(segment.events) ||
      !segment.events.every((event) => typeof event === 'string')
    ) return []
    return [{
      index: segment.index,
      start_ms: segment.start_ms,
      end_ms: segment.end_ms,
      text: segment.text,
      language: segment.language,
      emotion: segment.emotion,
      events: segment.events,
    }]
  })
  if (segments.length !== item.segments.length) return null
  return {
    text: item.text,
    language: item.language,
    emotion: item.emotion,
    events: item.events,
    audio_duration_ms: item.audio_duration_ms,
    segments,
  }
}

function languageLabel(language: string | null) {
  if (!language) return t('speech.undetermined')
  return ({ zh: t('language.zh'), en: t('language.en'), yue: t('language.yue'), ja: t('language.ja'), ko: t('language.ko') } as Record<string, string>)[language] ?? language
}

function emotionLabel(emotion: string | null) {
  if (!emotion) return t('speech.notDetected')
  return ({ HAPPY: t('emotion.happy'), SAD: t('emotion.sad'), ANGRY: t('emotion.angry'), NEUTRAL: t('emotion.neutral'), FEARFUL: t('emotion.fearful'), DISGUSTED: t('emotion.disgusted'), SURPRISED: t('emotion.surprised'), OTHER: t('emotion.other'), EMO_UNKNOWN: t('emotion.unknown') } as Record<string, string>)[emotion] ?? emotion
}

function eventLabel(event: string) {
  return ({ Speech: t('sound.speech'), BGM: t('sound.music'), Laughter: t('sound.laughter'), Applause: t('sound.applause'), Cry: t('sound.cry'), Sneeze: t('sound.sneeze'), Breath: t('sound.breath'), Cough: t('sound.cough'), Sing: t('sound.sing'), Speech_Noise: t('sound.noisySpeech') } as Record<string, string>)[event] ?? event
}

function formatTimestamp(milliseconds: number) {
  const totalSeconds = Math.max(0, milliseconds) / 1000
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds - minutes * 60
  return `${number(minutes)}:${number(seconds, 1).padStart(4, '0')}`
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value)
}
