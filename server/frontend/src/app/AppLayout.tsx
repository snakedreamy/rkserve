import { LanguageSwitch } from '../components/LanguageSwitch'
import { t, time } from '../i18n/core'
import { useI18n } from '../i18n/useI18n'
import { Icon, type IconName } from '../icons/Icon'
import { Link, NavLink, Outlet } from 'react-router-dom'
import { useSystem } from './SystemContext'
import { useAuth } from './AuthContext'

const navigation = () => [
  { to: '/npu', label: t('nav.compute'), icon: 'CircleGauge' },
  { to: '/plugins', label: t('nav.plugins'), icon: 'Box' },
  { to: '/apis', label: t('nav.api'), icon: 'Braces' },
  { to: '/events', label: t('nav.events'), icon: 'Activity' },
] satisfies Array<{ to: string; label: string; icon: IconName }>

export function AppLayout() {
  useI18n()
  const { topology, connected, updatedAt, refreshing, refresh, error, clearError } = useSystem()
  const { principal, disconnect } = useAuth()
  return (
    <div className="app-shell">
      <a className="skip-link" href="#main">{t('nav.skip')}</a>
      <aside className="app-rail">
        <Link className="brand" to="/npu" aria-label={t('nav.home')}>
          <span className="brand-chip" aria-hidden="true">RK</span>
          <span className="brand-word">
            <strong>RKServe</strong>
            <span>{t('app.edgeNpu')}</span>
          </span>
        </Link>
        <nav className="primary-nav" aria-label={t('nav.primary')}>
          {navigation().map(({ to, label, icon }) => (
            <NavLink
              key={to}
              to={to}
              aria-label={label}
              className={({ isActive }) => `nav-item ${isActive ? 'active' : ''}`}
            >
              <Icon name={icon} size={18} />
              <span>{label}</span>
            </NavLink>
          ))}
        </nav>
        <div className="rail-status">
          <LanguageSwitch />
          <div className={`connection ${connected ? 'online' : 'offline'}`} role="status">
            <span className="status-dot" />
            <span className="connection-copy">{connected ? t('connection.coreOnline') : t('connection.coreOffline')}</span>
          </div>
          <span className="node-pill" title={topology.device}>
            <Icon name="Cpu" size={13} />
            <strong>{topology.platform.toUpperCase()}</strong>
          </span>
          <span className="sync-copy">
            {updatedAt ? t('connection.synced', { time: time(updatedAt) }) : t('connection.waiting')}
          </span>
          <div className="rail-actions">
            <button
              className="identity-pill"
              type="button"
              onClick={disconnect}
              title={t('connection.disconnect', { name: principal.name })}
            >
              <span>{principal.name}</span>
              <small>{principal.role === 'api' ? t('common.api') : t('common.management')} · {principal.fingerprint}</small>
              <Icon name="Power" size={13} />
            </button>
            <button
              className="icon-button"
              type="button"
              aria-label={t('common.refreshData')}
              onClick={() => void refresh()}
              disabled={refreshing}
            >
              <Icon name="RefreshCw" size={15} className={refreshing ? 'spin' : ''} />
            </button>
          </div>
        </div>
      </aside>

      <header className="mobile-bar">
        <Link className="brand" to="/npu" aria-label={t('nav.home')}>
          <span className="brand-chip" aria-hidden="true">RK</span>
          <span className="brand-word">
            <strong>RKServe</strong>
            <span>{t('app.edgeNpu')}</span>
          </span>
        </Link>
        <div className="header-status">
          <LanguageSwitch />
          <div className={`connection ${connected ? 'online' : 'offline'}`} role="status">
            <span className="status-dot" />
            <span className="connection-copy">{connected ? t('connection.online') : t('connection.offline')}</span>
          </div>
          <button
            className="identity-pill"
            type="button"
            onClick={disconnect}
            title={t('connection.disconnect', { name: principal.name })}
          >
            <span>{principal.name}</span>
            <Icon name="Power" size={13} />
          </button>
          <button
            className="icon-button"
            type="button"
            aria-label={t('common.refreshData')}
            onClick={() => void refresh()}
            disabled={refreshing}
          >
            <Icon name="RefreshCw" size={15} className={refreshing ? 'spin' : ''} />
          </button>
        </div>
      </header>

      <div className="page">
        <main id="main">
          {error && (
            <div className="error-strip" role="alert">
              <Icon name="Unplug" size={15} />
              <span>{error}</span>
              <button type="button" onClick={clearError}>{t('common.close')}</button>
            </div>
          )}
          <Outlet />
        </main>
      </div>

      <nav className="bottom-nav" aria-label={t('nav.mobile')}>
        {navigation().map(({ to, label, icon }) => (
          <NavLink
            key={to}
            to={to}
            aria-label={label}
            className={({ isActive }) => `nav-item ${isActive ? 'active' : ''}`}
          >
            <Icon name={icon} size={18} />
            <span>{label}</span>
          </NavLink>
        ))}
      </nav>
    </div>
  )
}
