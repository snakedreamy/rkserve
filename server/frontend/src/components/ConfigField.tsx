import { t } from '../i18n/core'
import { useI18n } from '../i18n/useI18n'
import type { ConfigurationField } from '../types'

interface ConfigFieldProps {
  field: ConfigurationField
  value: string
  disabled?: boolean
  emptyLabel: string
  placeholder?: string
  numberStep?: string
  onChange: (value: string) => void
}

export function ConfigField({ field, value, disabled, emptyLabel, placeholder, numberStep, onChange }: ConfigFieldProps) {
  useI18n()
  if (field.kind === 'select') {
    return (
      <select value={value} disabled={disabled} onChange={(event) => onChange(event.target.value)}>
        {!field.required && <option value="">{emptyLabel}</option>}
        {field.options.map((option) => <option key={option} value={option}>{optionLabel(field, option)}</option>)}
      </select>
    )
  }

  if (field.kind === 'boolean') {
    return (
      <select value={value} disabled={disabled} onChange={(event) => onChange(event.target.value)}>
        <option value="">{emptyLabel}</option>
        <option value="true">{t('common.yes')}</option>
        <option value="false">{t('common.no')}</option>
      </select>
    )
  }

  return (
    <input
      type={field.kind === 'number' ? 'number' : 'text'}
      step={field.kind === 'number' ? numberStep : undefined}
      value={value}
      min={field.min ?? undefined}
      max={field.max ?? undefined}
      required={field.required}
      disabled={disabled}
      placeholder={placeholder}
      onChange={(event) => onChange(event.target.value)}
    />
  )
}

function optionLabel(field: ConfigurationField, option: string) {
  const localized = field.optionLabels?.[option]
  if (localized && localized !== option) return localized
  if (field.key === 'language') {
    return ({ auto: t('common.autoDetect'), zh: t('language.zh'), en: t('language.en'), yue: t('language.yue'), ja: t('language.ja'), ko: t('language.ko') } as Record<string, string>)[option] ?? option
  }
  if (field.key === 'text_normalization') {
    return option === 'withitn' ? t('common.enabled') : option === 'woitn' ? t('common.disabled') : option
  }
  if (field.options_from !== 'rknn_models') return option
  const segments = option.split('/')
  const fileName = segments.at(-1)?.replace(/\.rknn$/i, '') ?? option
  const directory = segments.length > 1 ? segments.at(-2) : null
  return directory ? `${fileName} · ${directory}` : fileName
}
