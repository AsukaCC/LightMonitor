import { useEffect, useState } from 'react'
import { AdminPage } from './pages/AdminPage'
import { PublicPage } from './pages/PublicPage'
import { applyTheme, getPreferredTheme, toggleTheme } from './theme'
import type { ThemeMode } from './types'
import { useI18n } from './i18n'
import './App.css'

function getRoute(): 'public' | 'admin' {
  const path = window.location.pathname.replace(/\/+$/, '') || '/'
  return path === '/admin' || path.startsWith('/admin/') ? 'admin' : 'public'
}

function App() {
  const [route, setRoute] = useState(getRoute)
  const [theme, setTheme] = useState<ThemeMode>(() => getPreferredTheme())
  const { language, t } = useI18n()

  useEffect(() => {
    applyTheme(theme)
  }, [theme])

  useEffect(() => {
    document.title = `LightMonitor · ${route === 'admin' ? t('管理后台') : t('公开监控')}`
  }, [language, route, t])

  useEffect(() => {
    const onPop = () => setRoute(getRoute())
    window.addEventListener('popstate', onPop)
    return () => window.removeEventListener('popstate', onPop)
  }, [])

  function handleToggleTheme() {
    setTheme((current) => toggleTheme(current))
  }

  if (route === 'admin') {
    return <AdminPage theme={theme} onToggleTheme={handleToggleTheme} />
  }

  return <PublicPage theme={theme} onToggleTheme={handleToggleTheme} />
}

export default App
