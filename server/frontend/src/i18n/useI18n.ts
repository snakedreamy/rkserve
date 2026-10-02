import { useSyncExternalStore } from 'react'
import { DEFAULT_LANGUAGE, getLanguage, subscribeLanguage } from './core'

export function useI18n() {
  return useSyncExternalStore(subscribeLanguage, getLanguage, () => DEFAULT_LANGUAGE)
}
