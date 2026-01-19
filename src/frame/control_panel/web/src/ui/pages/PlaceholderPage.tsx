const PlaceholderPage = ({ title, description, ctaLabel }: PlaceholderPageProps) => {
  return (
    <div className="cp-panel px-10 py-16 text-center">
      <div className="mx-auto max-w-2xl space-y-6">
        <h1 className="text-3xl font-semibold text-[var(--cp-ink)] sm:text-4xl">{title}</h1>
        <p className="text-sm leading-6 text-[var(--cp-muted)] sm:text-base">{description}</p>
        {ctaLabel ? (
          <button
            type="button"
            className="inline-flex items-center gap-2 rounded-full bg-[var(--cp-primary)] px-5 py-2 text-sm font-medium text-white shadow transition hover:bg-[var(--cp-primary-strong)]"
          >
            {ctaLabel}
          </button>
        ) : null}
      </div>
    </div>
  )
}

export default PlaceholderPage
