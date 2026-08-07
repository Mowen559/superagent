import { Network, Folder, Sparkles } from '@/lib/lucide-icons';
import { useTranslation } from 'react-i18next';

export type MobileTab = 'graph' | 'files' | 'chat';

interface MobileBottomNavProps {
  activeTab: MobileTab;
  onTabChange: (tab: MobileTab) => void;
}

const TAB_ORDER: MobileTab[] = ['graph', 'files', 'chat'];

const TAB_ICONS: Record<MobileTab, typeof Network> = {
  graph: Network,
  files: Folder,
  chat: Sparkles,
};

/**
 * Bottom tab bar for the mobile layout. Switches between the graph canvas,
 * the file tree and the AI chat panel.
 */
export default function MobileBottomNav({ activeTab, onTabChange }: MobileBottomNavProps) {
  const { t } = useTranslation(['common']);

  const labels: Record<MobileTab, string> = {
    graph: t('mobile.graph', 'Graph'),
    files: t('mobile.files', 'Files'),
    chat: t('mobile.chat', 'AI'),
  };

  return (
    <nav className="flex shrink-0 border-t border-border-subtle bg-deep">
      {TAB_ORDER.map((id) => {
        const Icon = TAB_ICONS[id];
        const active = activeTab === id;
        return (
          <button
            key={id}
            type="button"
            onClick={() => onTabChange(id)}
            aria-current={active ? 'page' : undefined}
            className={`relative flex flex-1 flex-col items-center justify-center gap-1 py-2.5 text-[10px] font-semibold tracking-[0.14em] uppercase transition-colors ${
              active ? 'text-accent' : 'text-text-muted hover:text-text-secondary'
            }`}
          >
            <Icon className="h-5 w-5" />
            {labels[id]}
            {active && (
              <span className="absolute top-0 left-1/2 h-px w-8 -translate-x-1/2 bg-accent" />
            )}
          </button>
        );
      })}
    </nav>
  );
}
