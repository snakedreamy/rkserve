import { en, zhCN } from './messages'
import type { ConfigurationField, PluginSummary } from '../types'

export type Language = 'en' | 'zh-CN'
export type TranslationKey = keyof typeof en
export type TranslationValues = Record<string, string | number>
export const LANGUAGE_STORAGE_KEY = 'rkserve.language'
export const DEFAULT_LANGUAGE: Language = 'en'
export const dictionaries = { en, 'zh-CN': zhCN } as const

export function isLanguage(value: unknown): value is Language {
  return value === 'en' || value === 'zh-CN'
}

// Browser language is deliberately ignored: every new installation starts in English.
export function readLanguage(storage?: Pick<Storage, 'getItem'>): Language {
  try {
    const value = (storage ?? window.localStorage).getItem(LANGUAGE_STORAGE_KEY)
    return isLanguage(value) ? value : DEFAULT_LANGUAGE
  } catch {
    return DEFAULT_LANGUAGE
  }
}

export function persistLanguage(language: Language, storage?: Pick<Storage, 'setItem'>): void {
  try {
    (storage ?? window.localStorage).setItem(LANGUAGE_STORAGE_KEY, language)
  } catch {
    // Private browsing and storage policies must not prevent an in-memory switch.
  }
}

export function translate(language: Language, key: TranslationKey, values: TranslationValues = {}): string {
  return dictionaries[language][key].replace(/\{(\w+)\}/g, (placeholder, name: string) =>
    Object.prototype.hasOwnProperty.call(values, name) ? String(values[name]) : placeholder,
  )
}

export function intlLocale(language: Language): string {
  return language === 'zh-CN' ? 'zh-CN' : 'en-US'
}

let language = readLanguage()
const listeners = new Set<() => void>()
export const getLanguage = () => language
export const subscribeLanguage = (listener: () => void) => {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

export function setLanguage(next: Language): void {
  if (!isLanguage(next)) return
  language = next
  persistLanguage(next)
  syncDocumentLanguage()
  listeners.forEach((listener) => listener())
}

export function syncDocumentLanguage(): void {
  if (typeof document === 'undefined') return
  document.documentElement.lang = language
  document.title = translate(language, 'app.title')
  document.querySelector('meta[name="description"]')?.setAttribute('content', translate(language, 'app.description'))
}

export const t = (key: TranslationKey, values?: TranslationValues) => translate(language, key, values)
export const number = (value: number, digits?: number) => new Intl.NumberFormat(intlLocale(language),
  digits === undefined ? undefined : { minimumFractionDigits: digits, maximumFractionDigits: digits },
).format(value)
export const dateTime = (value: Date | number, options?: Intl.DateTimeFormatOptions) =>
  new Intl.DateTimeFormat(intlLocale(language), options ?? { dateStyle: 'short', timeStyle: 'medium', hour12: false }).format(value)
export const time = (value: Date | number) => dateTime(value, { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false })

// Only frontend-owned errors are translated. Server errors, logs, and model output stay verbatim.
export class UiError extends Error {
  constructor(readonly key: TranslationKey, readonly values?: TranslationValues) {
    super(translate(DEFAULT_LANGUAGE, key, values))
    this.name = 'UiError'
  }
  localizedMessage(): string {
    return t(this.key, this.values)
  }
}
export function errorText(error: unknown, fallback: TranslationKey = 'error.operationFailed'): string {
  if (error instanceof UiError) return error.localizedMessage()
  return error instanceof Error ? error.message : typeof error === 'string' ? error : t(fallback)
}

export function pluginField(
  plugin: Pick<PluginSummary, 'translations'>,
  key: string,
  fallback: string,
  locale: Language = language,
): string {
  // English is canonical even if an external manifest supplies an English override.
  return locale === DEFAULT_LANGUAGE ? fallback : plugin.translations?.[locale]?.[key] ?? fallback
}

function localizedConfiguration(plugin: PluginSummary, field: ConfigurationField, prefix: string, locale: Language): ConfigurationField {
  return {
    ...field,
    label: pluginField(plugin, `${prefix}.label`, field.label, locale),
    description: pluginField(plugin, `${prefix}.description`, field.description, locale),
    optionLabels: Object.fromEntries(field.options.map((option) => [
      option, pluginField(plugin, `${prefix}.options.${option}`, option, locale),
    ])),
  }
}

// Build display-only copies. IDs, configuration keys/values, examples, and wire formats never change.
export function localizePlugin(plugin: PluginSummary, locale: Language = language): PluginSummary {
  return {
    ...plugin,
    name: pluginField(plugin, 'plugin.name', plugin.name, locale),
    configuration: plugin.configuration.map((field) => localizedConfiguration(plugin, field, `configuration.${field.key}`, locale)),
    capabilities: plugin.capabilities.map((capability) => {
      const prefix = `capabilities.${capability.id}`
      return {
        ...capability,
        name: pluginField(plugin, `${prefix}.name`, capability.name, locale),
        description: pluginField(plugin, `${prefix}.description`, capability.description, locale),
        parameters: capability.parameters.map((field) => localizedConfiguration(plugin, field, `${prefix}.parameters.${field.key}`, locale)),
        presentation: capability.presentation ? {
          ...capability.presentation,
          input_hint: capability.presentation.input_hint === null ? null : pluginField(plugin, `${prefix}.presentation.input_hint`, capability.presentation.input_hint, locale),
          input_placeholder: capability.presentation.input_placeholder === null ? null : pluginField(plugin, `${prefix}.presentation.input_placeholder`, capability.presentation.input_placeholder, locale),
        } : undefined,
      }
    }),
  }
}
