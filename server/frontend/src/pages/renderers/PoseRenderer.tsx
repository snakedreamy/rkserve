import { t, number } from '../../i18n/core'
import { useI18n } from '../../i18n/useI18n'
import { Icon } from '../../icons/Icon'

// ── Pose types ───────────────────────────────────────────────────────────────

export interface PoseResult {
  model: string
  poses: Array<{
    confidence: number
    box: { left: number; top: number; right: number; bottom: number }
    keypoints: Array<{
      id: number
      label: string
      x: number
      y: number
      confidence: number
    }>
  }>
}

// COCO 17-point skeleton edges
const poseEdges = [
  [5, 6],
  [5, 7],
  [7, 9],
  [6, 8],
  [8, 10],
  [5, 11],
  [6, 12],
  [11, 12],
  [11, 13],
  [13, 15],
  [12, 14],
  [14, 16],
  [0, 1],
  [0, 2],
  [1, 3],
  [2, 4],
] as const

const poseColors = ['#e07a3d', '#3aa8b5', '#d4a04a', '#e0564a', '#7b8cff', '#5ec49a']

// ── Canvas overlay ───────────────────────────────────────────────────────────

export function PoseOverlay({
  imageSize,
  pose,
}: {
  imageSize: { width: number; height: number }
  pose: PoseResult
}) {
  useI18n()
  const poses = clampPoses(pose, imageSize)
  if (!poses.length) return null
  return (
    <svg
      className="detection-overlay"
      viewBox={`0 0 ${imageSize.width} ${imageSize.height}`}
      aria-hidden="true"
    >
      {poses.map((person, personIndex) => {
        const color = poseColors[personIndex % poseColors.length]
        const visible = new Map(
          person.keypoints
            .filter((point) => point.confidence >= 0.3)
            .map((point) => [point.id, point]),
        )
        return (
          <g key={personIndex}>
            <rect
              x={person.box.left}
              y={person.box.top}
              width={person.box.right - person.box.left}
              height={person.box.bottom - person.box.top}
              fill="none"
              stroke={color}
              strokeWidth="2"
              vectorEffect="non-scaling-stroke"
              opacity="0.75"
            />
            {poseEdges.map(([startId, endId]) => {
              const start = visible.get(startId)
              const end = visible.get(endId)
              return start && end ? (
                <line
                  key={`${startId}-${endId}`}
                  x1={start.x}
                  y1={start.y}
                  x2={end.x}
                  y2={end.y}
                  stroke={color}
                  strokeWidth="3"
                  vectorEffect="non-scaling-stroke"
                />
              ) : null
            })}
            {[...visible.values()].map((point) => (
              <circle
                key={point.id}
                cx={point.x}
                cy={point.y}
                r="4"
                fill={color}
                stroke="#14181f"
                strokeWidth="1.5"
                vectorEffect="non-scaling-stroke"
              />
            ))}
          </g>
        )
      })}
    </svg>
  )
}

// ── Result panel ─────────────────────────────────────────────────────────────

export function PoseOutput({
  pose,
  outputUrl,
  downloadName,
  rawText,
}: {
  pose: PoseResult
  outputUrl: string | null
  downloadName: string | undefined
  rawText: string | null
}) {
  useI18n()
  return (
    <div className="detection-result">
      <div className="result-summary">
        <strong>{pose.poses.length ? t('result.people', { count: number(pose.poses.length) }) : t('result.noPeople')}</strong>
        <span>{pose.model}</span>
      </div>
      {pose.poses.length > 0 && (
        <div className="detection-list">
          {pose.poses.map((person, index) => (
            <div key={index}>
              <i style={{ backgroundColor: poseColors[index % poseColors.length] }} />
              <strong>{t('result.person', { index: number(index + 1) })}</strong>
              <span>{number(person.confidence * 100, 1)}%</span>
              <code>
                {t('result.points', { count: number(person.keypoints.filter((point) => point.confidence >= 0.3).length) })}
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

export function parsePoseResult(value: unknown): PoseResult | null {
  if (!value || typeof value !== 'object') return null
  const candidate = value as { model?: unknown; poses?: unknown }
  if (typeof candidate.model !== 'string' || !Array.isArray(candidate.poses)) return null
  const poses = candidate.poses.flatMap((value) => {
    if (!value || typeof value !== 'object') return []
    const item = value as { confidence?: unknown; box?: unknown; keypoints?: unknown }
    if (
      typeof item.confidence !== 'number' ||
      !Number.isFinite(item.confidence) ||
      !item.box ||
      typeof item.box !== 'object' ||
      !Array.isArray(item.keypoints)
    ) {
      return []
    }
    const box = item.box as Record<string, unknown>
    if (![box.left, box.top, box.right, box.bottom].every(isFiniteNumber)) return []
    const keypoints = item.keypoints.flatMap((value) => {
      if (!value || typeof value !== 'object') return []
      const point = value as Record<string, unknown>
      if (
        typeof point.id !== 'number' ||
        typeof point.label !== 'string' ||
        !isFiniteNumber(point.x) ||
        !isFiniteNumber(point.y) ||
        !isFiniteNumber(point.confidence)
      ) {
        return []
      }
      return [{
        id: point.id,
        label: point.label,
        x: point.x,
        y: point.y,
        confidence: point.confidence,
      }]
    })
    if (keypoints.length !== 17) return []
    return [{
      confidence: item.confidence,
      box: {
        left: box.left as number,
        top: box.top as number,
        right: box.right as number,
        bottom: box.bottom as number,
      },
      keypoints,
    }]
  })
  return { model: candidate.model, poses }
}

export function clampPoses(
  result: PoseResult,
  size: { width: number; height: number },
): PoseResult['poses'] {
  return result.poses.flatMap((pose) => {
    const left = Math.max(0, Math.min(size.width, pose.box.left))
    const top = Math.max(0, Math.min(size.height, pose.box.top))
    const right = Math.max(0, Math.min(size.width, pose.box.right))
    const bottom = Math.max(0, Math.min(size.height, pose.box.bottom))
    if (right <= left || bottom <= top) return []
    return [{
      ...pose,
      box: { left, top, right, bottom },
      keypoints: pose.keypoints.map((point) => ({
        ...point,
        x: Math.max(0, Math.min(size.width, point.x)),
        y: Math.max(0, Math.min(size.height, point.y)),
      })),
    }]
  })
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value)
}
