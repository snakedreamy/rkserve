import { LanguageSwitch } from '../components/LanguageSwitch'
import { t, UiError, errorText } from '../i18n/core'
import { useI18n } from '../i18n/useI18n'
import { createContext, type FormEvent, type ReactNode, useCallback, useContext, useEffect, useState } from 'react'
import { AUTH_REQUIRED_EVENT, api, clearApiKey, getApiKey, setApiKey } from '../api'
import { Icon } from '../icons/Icon'
import type { ApiPrincipal } from '../types'

interface AuthContextValue {
  principal: ApiPrincipal
  disconnect: () => void
}

const AuthContext = createContext<AuthContextValue | null>(null)

function requireManagementKey(identity: ApiPrincipal): ApiPrincipal {
  if (identity.role !== 'management') {
    throw new UiError('error.managementKey')
  }
  return identity
}

export function useAuth() {
  const context = useContext(AuthContext)
  if (!context) throw new Error('useAuth must be used inside AuthProvider')
  return context
}

export function AuthProvider({ children }: { children: ReactNode }) {
  useI18n()
  const [principal, setPrincipal] = useState<ApiPrincipal | null>(null)
  const [checking, setChecking] = useState(Boolean(getApiKey()))
  const [key, setKey] = useState('')
  const [error, setError] = useState<unknown>(null)

  const disconnect = useCallback(() => {
    clearApiKey()
    setPrincipal(null)
    setKey('')
    setChecking(false)
  }, [])

  useEffect(() => {
    const requireAuth = () => disconnect()
    window.addEventListener(AUTH_REQUIRED_EVENT, requireAuth)
    return () => window.removeEventListener(AUTH_REQUIRED_EVENT, requireAuth)
  }, [disconnect])

  useEffect(() => {
    if (!getApiKey()) return
    let active = true
    api.whoami()
      .then(async (identity) => {
        const principal = requireManagementKey(identity)
        await api.topology()
        if (active) {
          setPrincipal(principal)
          setError(null)
        }
      })
      .catch((cause) => {
        if (active) {
          clearApiKey()
          setError(cause instanceof Error ? cause : new UiError('error.verifyKey'))
        }
      })
      .finally(() => {
        if (active) setChecking(false)
      })
    return () => { active = false }
  }, [])

  const connect = async (event: FormEvent) => {
    event.preventDefault()
    const value = key.trim()
    if (!value) {
      setError(new UiError('error.enterKey'))
      return
    }
    setChecking(true)
    setError(null)
    setApiKey(value)
    try {
      const identity = requireManagementKey(await api.whoami())
      await api.topology()
      setPrincipal(identity)
      setKey('')
    } catch (cause) {
      clearApiKey()
      setError(cause instanceof Error ? cause : new UiError('error.invalidKey'))
    } finally {
      setChecking(false)
    }
  }

  if (!principal) {
    return (
      <main className="auth-shell">
        <div className="auth-language"><LanguageSwitch /></div>
        <section className="auth-panel" aria-labelledby="auth-title">
          <div className="auth-mark">
            <span>RK</span>
            <small>{t('app.npuConsole')}</small>
          </div>
          <div className="auth-copy">
            <span className="eyebrow">{t('auth.access')}</span>
            <h1 id="auth-title">{t('auth.title')}</h1>
            <p>{t('auth.description')}</p>
            <dl>
              <div><dt>{t('auth.transport')}</dt><dd>{t('auth.https')}</dd></div>
              <div><dt>{t('auth.validation')}</dt><dd>{t('auth.perRequest')}</dd></div>
            </dl>
          </div>
          <form className="auth-form" onSubmit={(event) => void connect(event)}>
            <label htmlFor="api-key">{t('auth.managementKey')}</label>
            <div className="auth-input">
              <Icon name="KeyRound" size={16} />
              <input
                id="api-key"
                type="password"
                value={key}
                onChange={(event) => setKey(event.target.value)}
                placeholder={t('auth.placeholder')}
                autoComplete="off"
                autoFocus
                disabled={checking}
              />
            </div>
            {Boolean(error) && (
              <span className="auth-error" role="alert">
                <Icon name="CircleAlert" size={14} />
                {errorText(error)}
              </span>
            )}
            <button type="submit" disabled={checking}>
              {checking ? <Icon name="LoaderCircle" size={15} className="spin" /> : <Icon name="Unplug" size={15} />}
              {checking ? t('auth.verifying') : t('auth.connect')}
            </button>
          </form>
        </section>
      </main>
    )
  }

  return <AuthContext.Provider value={{ principal, disconnect }}>{children}</AuthContext.Provider>
}
