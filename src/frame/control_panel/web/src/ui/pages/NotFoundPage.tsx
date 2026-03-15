import { Link } from 'react-router-dom'

import { useI18n } from '@/i18n'

const NotFoundPage = () => {
  const { t } = useI18n()

  return (
    <div className="flex min-h-screen items-center justify-center px-4 py-8">
      <div className="cp-panel w-full max-w-xl space-y-5 px-8 py-10 text-center">
        <p className="text-xs font-semibold uppercase tracking-[0.18em] text-[var(--cp-muted)]">404</p>
        <h1 className="text-3xl font-semibold text-[var(--cp-ink)]">{t('notFound.title', 'Page Not Found')}</h1>
        <p className="text-sm leading-6 text-[var(--cp-muted)]">{t('notFound.description', 'The page you requested does not exist or has been moved.')}</p>
        <div className="flex flex-wrap items-center justify-center gap-3 pt-2">
          <Link
            to="/"
            className="inline-flex items-center rounded-xl bg-[var(--cp-primary)] px-4 py-2 text-sm font-semibold text-white transition hover:bg-[var(--cp-primary-strong)]"
          >
            {t('notFound.goHome', 'Go to Home')}
          </Link>
          <Link
            to="/login"
            className="inline-flex items-center rounded-xl border border-[var(--cp-border)] bg-white px-4 py-2 text-sm font-semibold text-[var(--cp-ink)] transition hover:border-[var(--cp-primary)] hover:text-[var(--cp-primary)]"
          >
            {t('notFound.goLogin', 'Go to Login')}
          </Link>
        </div>
      </div>
    </div>
  )
}

export default NotFoundPage
