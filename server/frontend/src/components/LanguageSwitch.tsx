import { setLanguage, t, type Language } from '../i18n/core'
import { useI18n } from '../i18n/useI18n'

export function LanguageSwitch() {
  const language = useI18n()
  return (
    <label className="language-switch">
      <span>{t('language.label')}</span>
      <select aria-label={t('language.label')} value={language} onChange={(event) => setLanguage(event.target.value as Language)}>
        <option value="en" lang="en">English</option>
        <option value="zh-CN" lang="zh-CN">简体中文</option>
      </select>
    </label>
  )
}
