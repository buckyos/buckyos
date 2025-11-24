const PlaceholderPage = ({ title, description, ctaLabel }: PlaceholderPageProps) => {
  return (
    <div className="rounded-3xl border border-slate-900/60 bg-slate-900/40 px-10 py-16 text-center shadow-lg shadow-black/20 backdrop-blur">
      <div className="mx-auto max-w-2xl space-y-6">
        <h1 className="text-3xl font-semibold text-white sm:text-4xl">{title}</h1>
        <p className="text-sm leading-6 text-slate-400 sm:text-base">{description}</p>
        {ctaLabel ? (
          <button
            type="button"
            className="inline-flex items-center gap-2 rounded-full bg-sky-500 px-5 py-2 text-sm font-medium text-white shadow transition hover:bg-sky-400"
          >
            {ctaLabel}
          </button>
        ) : null}
      </div>
    </div>
  )
}

export default PlaceholderPage
