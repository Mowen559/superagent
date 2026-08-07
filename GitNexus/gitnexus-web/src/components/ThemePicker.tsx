import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useTheme, PRESETS } from '../themes';
import type { HeadingFont } from '../themes';

/**
 * Theme picker dropdown — ported from Understand-Anything dashboard.
 * Adapted to GitNexus: uses react-i18next (with English defaults) instead of
 * UA's I18nContext, and drops the language section (GN has LanguageSwitcher).
 */
export function ThemePicker() {
  const { config, preset, setPreset, setAccent, setHeadingFont } = useTheme();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const { t } = useTranslation(['common']);

  // Close on outside click
  useEffect(() => {
    if (!open) return;
    function handleClick(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener('mousedown', handleClick);
    return () => document.removeEventListener('mousedown', handleClick);
  }, [open]);

  // Close on Escape
  useEffect(() => {
    if (!open) return;
    function handleKey(e: KeyboardEvent) {
      if (e.key === 'Escape') setOpen(false);
    }
    document.addEventListener('keydown', handleKey);
    return () => document.removeEventListener('keydown', handleKey);
  }, [open]);

  const handlePreset = useCallback(
    (id: string) => {
      setPreset(id as Parameters<typeof setPreset>[0]);
    },
    [setPreset],
  );

  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => setOpen((v) => !v)}
        className="flex items-center gap-1.5 rounded px-2 py-1 text-xs text-text-secondary transition-colors hover:text-text-primary"
        title={t('theme.changeTheme', 'Change theme')}
      >
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <circle cx="12" cy="12" r="10" />
          <path d="M12 2a7 7 0 0 0 0 14 4 4 0 0 1 0 8 10 10 0 0 0 0-20z" />
          <circle cx="8" cy="10" r="1.5" fill="currentColor" />
          <circle cx="12" cy="7" r="1.5" fill="currentColor" />
          <circle cx="16" cy="10" r="1.5" fill="currentColor" />
        </svg>
        <span className="hidden sm:inline">{t('theme.theme', 'Theme')}</span>
      </button>

      {open && (
        <div className="absolute top-full right-0 z-50 mt-2 w-64 space-y-3 rounded-lg border border-border-default bg-elevated p-3 shadow-xl">
          {/* Presets */}
          <div>
            <div className="mb-2 text-[10px] font-semibold tracking-wider text-text-muted uppercase">
              {t('theme.theme', 'Theme')}
            </div>
            <div className="space-y-1">
              {PRESETS.map((p) => (
                <button
                  key={p.id}
                  onClick={() => handlePreset(p.id)}
                  className={`flex w-full items-center gap-2.5 rounded px-2.5 py-1.5 text-xs transition-colors ${
                    p.id === config.presetId
                      ? 'bg-accent/15 text-accent'
                      : 'text-text-secondary hover:bg-hover hover:text-text-primary'
                  }`}
                >
                  {/* Color preview dots */}
                  <div className="flex gap-1">
                    <span
                      className="h-3 w-3 rounded-full border border-border-subtle"
                      style={{ backgroundColor: p.colors.root }}
                    />
                    <span
                      className="h-3 w-3 rounded-full border border-border-subtle"
                      style={{ backgroundColor: p.colors.surface }}
                    />
                    <span
                      className="h-3 w-3 rounded-full border border-border-subtle"
                      style={{
                        backgroundColor:
                          p.accentSwatches.find((s) => s.id === p.defaultAccentId)?.accent ??
                          p.accentSwatches[0].accent,
                      }}
                    />
                  </div>
                  <span>{p.name}</span>
                  {p.id === config.presetId && (
                    <svg
                      className="ml-auto h-3.5 w-3.5 text-accent"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="3"
                    >
                      <polyline points="20 6 9 17 4 12" />
                    </svg>
                  )}
                </button>
              ))}
            </div>
          </div>

          {/* Accent swatches */}
          <div>
            <div className="mb-2 text-[10px] font-semibold tracking-wider text-text-muted uppercase">
              {t('theme.accentColor', 'Accent color')}
            </div>
            <div className="flex flex-wrap gap-2">
              {preset.accentSwatches.map((swatch) => (
                <button
                  key={swatch.id}
                  onClick={() => setAccent(swatch.id)}
                  className={`h-6 w-6 rounded-full transition-transform hover:scale-110 ${
                    swatch.id === config.accentId
                      ? 'ring-2 ring-text-primary ring-offset-1 ring-offset-void'
                      : ''
                  }`}
                  style={{ backgroundColor: swatch.accent }}
                  title={swatch.name}
                />
              ))}
            </div>
          </div>

          {/* Heading font */}
          <div>
            <div className="mb-2 text-[10px] font-semibold tracking-wider text-text-muted uppercase">
              {t('theme.headingFont', 'Heading font')}
            </div>
            <div className="flex gap-1">
              {[
                { id: 'serif' as HeadingFont, label: t('theme.serif', 'Serif') },
                { id: 'sans' as HeadingFont, label: t('theme.sans', 'Sans') },
                { id: 'mono' as HeadingFont, label: t('theme.mono', 'Mono') },
              ].map((opt) => (
                <button
                  key={opt.id}
                  onClick={() => setHeadingFont(opt.id)}
                  className={`flex-1 rounded px-2 py-1.5 text-xs transition-colors ${
                    (config.headingFont ?? 'serif') === opt.id
                      ? 'bg-accent/15 text-accent'
                      : 'text-text-secondary hover:bg-hover hover:text-text-primary'
                  }`}
                  style={{
                    fontFamily:
                      opt.id === 'serif'
                        ? 'var(--font-serif)'
                        : opt.id === 'mono'
                          ? 'var(--font-mono)'
                          : 'var(--font-sans)',
                  }}
                >
                  {opt.label}
                </button>
              ))}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
