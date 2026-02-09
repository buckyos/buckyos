import type { ReactNode } from 'react'

type IconProps = {
  name: IconName
  className?: string
  title?: string
}

const icons: Record<IconName, ReactNode> = {
  desktop: (
    <>
      <rect x="4" y="5" width="16" height="11" rx="2" />
      <path d="M8 21h8" />
      <path d="M12 16v5" />
    </>
  ),
  dashboard: (
    <>
      <rect x="3" y="3" width="8" height="8" rx="1.5" />
      <rect x="13" y="3" width="8" height="5" rx="1.5" />
      <rect x="13" y="10" width="8" height="11" rx="1.5" />
      <rect x="3" y="13" width="8" height="8" rx="1.5" />
    </>
  ),
  users: (
    <>
      <circle cx="9" cy="8" r="3" />
      <path d="M4 19c0-2.5 2.2-4.5 5-4.5" />
      <circle cx="17" cy="9" r="2.5" />
      <path d="M14.5 19c0-2 1.7-3.6 3.8-3.6" />
    </>
  ),
  storage: (
    <>
      <rect x="4" y="5" width="16" height="4" rx="1.5" />
      <rect x="4" y="10" width="16" height="4" rx="1.5" />
      <rect x="4" y="15" width="16" height="4" rx="1.5" />
      <circle cx="7" cy="7" r="0.8" fill="currentColor" stroke="none" />
      <circle cx="7" cy="12" r="0.8" fill="currentColor" stroke="none" />
      <circle cx="7" cy="17" r="0.8" fill="currentColor" stroke="none" />
    </>
  ),
  apps: (
    <>
      <rect x="4" y="4" width="7" height="7" rx="1.5" />
      <rect x="13" y="4" width="7" height="7" rx="1.5" />
      <rect x="4" y="13" width="7" height="7" rx="1.5" />
      <rect x="13" y="13" width="7" height="7" rx="1.5" />
    </>
  ),
  settings: (
    <>
      <circle cx="12" cy="12" r="3.2" />
      <path d="M4 12h2M18 12h2M12 4v2M12 18v2M6.3 6.3l1.4 1.4M16.3 16.3l1.4 1.4M6.3 17.7l1.4-1.4M16.3 7.7l1.4-1.4" />
    </>
  ),
  bell: (
    <>
      <path d="M6 9a6 6 0 1 1 12 0v4l2 2H4l2-2z" />
      <path d="M9.5 19a2.5 2.5 0 0 0 5 0" />
    </>
  ),
  signout: (
    <>
      <path d="M10 6H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h4" />
      <path d="M14 16l4-4-4-4" />
      <path d="M18 12H9" />
    </>
  ),
  alert: (
    <>
      <path d="M12 3l9 16H3l9-16z" />
      <path d="M12 9v4" />
      <circle cx="12" cy="16.5" r="0.8" fill="currentColor" stroke="none" />
    </>
  ),
  spark: (
    <>
      <path d="M12 3l2.2 4.6L19 10l-4.8 2.4L12 17l-2.2-4.6L5 10l4.8-2.4z" />
    </>
  ),
  cpu: (
    <>
      <rect x="7" y="7" width="10" height="10" rx="2" />
      <path d="M9 1v3M15 1v3M9 20v3M15 20v3M1 9h3M1 15h3M20 9h3M20 15h3" />
    </>
  ),
  memory: (
    <>
      <rect x="4" y="6" width="16" height="12" rx="2" />
      <path d="M8 10v4M12 10v4M16 10v4" />
    </>
  ),
  network: (
    <>
      <circle cx="5" cy="12" r="2" />
      <circle cx="19" cy="6" r="2" />
      <circle cx="19" cy="18" r="2" />
      <path d="M7 12h6M13 12l4-4M13 12l4 4" />
    </>
  ),
  package: (
    <>
      <path d="M3 7l9-4 9 4-9 4-9-4z" />
      <path d="M3 7v10l9 4 9-4V7" />
      <path d="M12 11v10" />
    </>
  ),
  shield: (
    <>
      <path d="M12 3l7 3v6c0 4.5-3 7.5-7 9-4-1.5-7-4.5-7-9V6l7-3z" />
      <path d="M9.5 12.5l2 2 3.5-3.5" />
    </>
  ),
  link: (
    <>
      <path d="M9 12a3 3 0 0 1 3-3h3" />
      <path d="M15 12a3 3 0 0 1-3 3H9" />
      <path d="M8 7l2-2M16 19l-2 2" />
    </>
  ),
  activity: (
    <>
      <path d="M4 12h4l2-4 4 8 2-4h4" />
    </>
  ),
  drive: (
    <>
      <rect x="3" y="6" width="18" height="12" rx="2.5" />
      <circle cx="17" cy="12" r="1" fill="currentColor" stroke="none" />
      <path d="M7 12h6" />
    </>
  ),
  chart: (
    <>
      <path d="M4 19h16" />
      <rect x="6" y="11" width="3" height="6" rx="0.8" />
      <rect x="11" y="7" width="3" height="10" rx="0.8" />
      <rect x="16" y="9" width="3" height="8" rx="0.8" />
    </>
  ),
  server: (
    <>
      <rect x="4" y="4" width="16" height="6" rx="1.5" />
      <rect x="4" y="14" width="16" height="6" rx="1.5" />
      <circle cx="8" cy="7" r="0.8" fill="currentColor" stroke="none" />
      <circle cx="8" cy="17" r="0.8" fill="currentColor" stroke="none" />
    </>
  ),
}

const Icon = ({ name, className, title }: IconProps) => {
  const labelled = Boolean(title)
  return (
    <svg
      viewBox="0 0 24 24"
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={!labelled}
      role={labelled ? 'img' : 'presentation'}
    >
      {labelled ? <title>{title}</title> : null}
      {icons[name]}
    </svg>
  )
}

export default Icon
