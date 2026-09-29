type Props = {
  lang: 'pl' | 'en'
  onAllow: () => void
  onDecline: () => void
}

export function ErrorReportingConsentModal({ lang, onAllow, onDecline }: Props) {
  const pl = lang === 'pl'

  return (
    <div
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 300,
        background: 'var(--c-overlay, rgba(0,0,0,0.55))',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        padding: 24,
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="error-consent-title"
        style={{
          width: '100%',
          maxWidth: 420,
          background: 'var(--c-surface)',
          border: '0.5px solid var(--c-border)',
          borderRadius: 14,
          padding: '28px 24px 20px',
          boxShadow: '0 16px 48px rgba(0,0,0,0.35)',
        }}
      >
        <div
          id="error-consent-title"
          style={{ fontSize: 17, fontWeight: 600, color: 'var(--c-text)', marginBottom: 12, letterSpacing: -0.2 }}
        >
          {pl ? 'Pomóż nam ulepszać ideiMx' : 'Help us improve ideiMx'}
        </div>
        <p
          style={{
            fontSize: 13,
            lineHeight: 1.55,
            color: 'var(--c-text-secondary)',
            margin: '0 0 22px',
          }}
        >
          {pl
            ? 'Czy zgadzasz się na automatyczne przesyłanie anonimowych raportów o błędach i awariach aplikacji? Pomaga nam to szybko namierzać i naprawiać usterki. Możesz to w każdej chwili zmienić w Ustawieniach.'
            : 'Allow automatic sending of anonymous error and crash reports? It helps us find and fix issues faster. You can change this anytime in Settings.'}
        </p>
        <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
          <button
            type="button"
            onClick={onDecline}
            style={{
              flex: 1,
              padding: '10px 12px',
              borderRadius: 8,
              border: '0.5px solid var(--c-border-soft)',
              background: 'transparent',
              color: 'var(--c-text-muted)',
              fontSize: 13,
              cursor: 'pointer',
            }}
          >
            {pl ? 'Nie, dziękuję' : 'No thanks'}
          </button>
          <button
            type="button"
            onClick={onAllow}
            style={{
              flex: 1,
              padding: '10px 12px',
              borderRadius: 8,
              border: 'none',
              background: 'var(--c-accent, #6366f1)',
              color: '#fff',
              fontSize: 13,
              fontWeight: 500,
              cursor: 'pointer',
            }}
          >
            {pl ? 'Zezwól' : 'Allow'}
          </button>
        </div>
      </div>
    </div>
  )
}
