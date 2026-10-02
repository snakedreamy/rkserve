import { t, number } from '../../i18n/core'
import { useI18n } from '../../i18n/useI18n'
import type { CapabilityDefinition } from '../../types'
import { Icon } from '../../icons/Icon'
import { useState } from 'react'

// ── Detection types ─────────────────────────────────────────────────────────

export interface DetectionResult {
  model: string
  detections: Array<{
    class_id: number
    label: string
    confidence: number
    box: { left: number; top: number; right: number; bottom: number }
  }>
}

// ── Canvas overlay ───────────────────────────────────────────────────────────

const detectionColors = ['#e07a3d', '#3aa8b5', '#d4a04a', '#e0564a', '#7b8cff', '#5ec49a']

export function DetectionOverlay({
  imageSize,
  detection,
}: {
  imageSize: { width: number; height: number }
  detection: DetectionResult
}) {
  useI18n()
  const items = clampDetections(detection, imageSize)
  if (!items.length) return null
  const fontSize = Math.max(14, imageSize.width / 48)
  return (
    <svg
      className="detection-overlay"
      viewBox={`0 0 ${imageSize.width} ${imageSize.height}`}
      aria-hidden="true"
    >
      {items.map((item, index) => {
        const color = detectionColors[Math.abs(item.class_id) % detectionColors.length]
        return (
          <g key={`${item.label}-${index}`}>
            <rect
              x={item.box.left}
              y={item.box.top}
              width={item.box.right - item.box.left}
              height={item.box.bottom - item.box.top}
              fill="none"
              stroke={color}
              strokeWidth="2"
              vectorEffect="non-scaling-stroke"
            />
            <text
              x={item.box.left + 4}
              y={Math.max(item.box.top - 5, fontSize)}
              fill={color}
              stroke="#14181f"
              strokeWidth={fontSize / 4}
              paintOrder="stroke"
              fontSize={fontSize}
              fontWeight="600"
            >
              {item.label} {number(item.confidence * 100, 0)}%
            </text>
          </g>
        )
      })}
    </svg>
  )
}

// ── Result panel ─────────────────────────────────────────────────────────────

export function DetectionOutput({
  detection,
  outputUrl,
  downloadName,
  rawText,
}: {
  detection: DetectionResult
  outputUrl: string | null
  downloadName: string | undefined
  rawText: string | null
}) {
  useI18n()
  return (
    <div className="detection-result">
      <div className="result-summary">
        <strong>
          {detection.detections.length
            ? t('result.targets', { count: number(detection.detections.length) })
            : t('result.noTargets')}
        </strong>
        <span>{detection.model}</span>
      </div>
      {detection.detections.length > 0 && (
        <div className="detection-list">
          {detection.detections.map((item, index) => (
            <div key={`${item.label}-${index}`}>
              <i
                style={{
                  backgroundColor:
                    detectionColors[Math.abs(item.class_id) % detectionColors.length],
                }}
              />
              <strong>{item.label}</strong>
              <span>{number(item.confidence * 100, 1)}%</span>
              <code>
                {item.box.left},{item.box.top} → {item.box.right},{item.box.bottom}
              </code>
            </div>
          ))}
        </div>
      )}
      <div className="result-actions">
        {outputUrl && (
          <a href={outputUrl} download={downloadName}>
            <Icon name="Download" size={14} />{t('common.downloadJson')}
          </a>
        )}
      </div>
      {rawText && (
        <details className="raw-output">
          <summary>{t('common.rawJson')}</summary>
          <pre>{rawText}</pre>
        </details>
      )}
    </div>
  )
}

// ── Helpers ──────────────────────────────────────────────────────────────────

export function parseDetectionResult(value: unknown): DetectionResult | null {
  if (!value || typeof value !== 'object') return null
  const candidate = value as { model?: unknown; detections?: unknown }
  if (typeof candidate.model !== 'string' || !Array.isArray(candidate.detections)) return null
  const detections = candidate.detections.flatMap((value) => {
    if (!value || typeof value !== 'object') return []
    const item = value as {
      class_id?: unknown
      label?: unknown
      confidence?: unknown
      box?: unknown
    }
    if (
      typeof item.class_id !== 'number' ||
      typeof item.label !== 'string' ||
      typeof item.confidence !== 'number' ||
      !item.box ||
      typeof item.box !== 'object'
    ) {
      return []
    }
    const box = item.box as {
      left?: unknown
      top?: unknown
      right?: unknown
      bottom?: unknown
    }
    if (
      ![item.class_id, item.confidence, box.left, box.top, box.right, box.bottom].every(
        (number) => typeof number === 'number' && Number.isFinite(number),
      )
    ) {
      return []
    }
    return [
      {
        class_id: item.class_id,
        label: item.label,
        confidence: item.confidence,
        box: {
          left: box.left as number,
          top: box.top as number,
          right: box.right as number,
          bottom: box.bottom as number,
        },
      },
    ]
  })
  return { model: candidate.model, detections }
}

export function clampDetections(
  result: DetectionResult,
  size: { width: number; height: number },
): DetectionResult['detections'] {
  return result.detections.flatMap((item) => {
    const left = Math.max(0, Math.min(size.width, item.box.left))
    const top = Math.max(0, Math.min(size.height, item.box.top))
    const right = Math.max(0, Math.min(size.width, item.box.right))
    const bottom = Math.max(0, Math.min(size.height, item.box.bottom))
    return right > left && bottom > top
      ? [{ ...item, box: { left, top, right, bottom } }]
      : []
  })
}

// Suppress unused-import lint for CapabilityDefinition used by parent page
export type { CapabilityDefinition }
