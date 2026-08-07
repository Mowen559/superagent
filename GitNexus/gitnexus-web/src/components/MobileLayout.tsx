import { useEffect, useState, type RefObject } from 'react';
import { Settings, HelpCircle, X } from '@/lib/lucide-icons';
import { useTranslation } from 'react-i18next';
import { useAppState } from '../hooks/useAppState';
import { GraphCanvas, type GraphCanvasHandle } from './GraphCanvas';
import { FileTreePanel } from './FileTreePanel';
import { RightPanel } from './RightPanel';
import { CodeReferencesPanel } from './CodeReferencesPanel';
import UAToolbar from './UAToolbar';
import { SettingsPanel } from './SettingsPanel';
import { LanguageSwitcher } from './LanguageSwitcher';
import { ThemePicker } from './ThemePicker';
import MobileBottomNav, { type MobileTab } from './MobileBottomNav';
import type { BackendRepo } from '../services/backend-client';

interface MobileLayoutProps {
  graphCanvasRef: RefObject<GraphCanvasHandle | null>;
  availableRepos: BackendRepo[];
  onSwitchRepo: (repoName: string) => void;
}

/**
 * Mobile layout — rendered instead of the desktop three-column view when the
 * viewport is narrow (see useIsMobile). Reuses the existing self-contained GN
 * panels (GraphCanvas, FileTreePanel, RightPanel, UAToolbar) and switches
 * between them through a bottom tab bar. A slide-in drawer hosts the secondary
 * controls (repo switch, language, theme, settings, help) that live in the
 * desktop Header.
 */
export default function MobileLayout({
  graphCanvasRef,
  availableRepos,
  onSwitchRepo,
}: MobileLayoutProps) {
  const { t } = useTranslation(['common', 'header']);
  const {
    graph,
    projectName,
    selectedNode,
    isRightPanelOpen,
    setRightPanelOpen,
    isSettingsPanelOpen,
    setSettingsPanelOpen,
    setHelpDialogBoxOpen,
    refreshLLMSettings,
    initializeAgent,
    codeReferences,
    isCodePanelOpen,
  } = useAppState();

  const [activeTab, setActiveTab] = useState<MobileTab>('graph');
  const [drawerOpen, setDrawerOpen] = useState(false);

  // RightPanel renders null unless the chat panel is flagged open, so make the
  // chat tab self-activate when selected.
  useEffect(() => {
    if (activeTab === 'chat') setRightPanelOpen(true);
  }, [activeTab, setRightPanelOpen]);

  // Selecting a node from the file tree should reveal the graph it lands on.
  const handleFocusNode = (nodeId: string) => {
    graphCanvasRef.current?.focusNode(nodeId);
    setActiveTab('graph');
  };

  const handleSettingsSaved = () => {
    refreshLLMSettings();
    initializeAgent();
  };

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-void text-text-primary">
      {/* Top bar */}
      <header className="flex h-12 shrink-0 items-center gap-2 border-b border-border-subtle bg-deep px-3">
        <button
          type="button"
          onClick={() => setDrawerOpen(true)}
          aria-label={t('common:menu', 'Menu')}
          className="-ml-1 flex h-9 w-9 items-center justify-center rounded-lg text-text-secondary transition-colors hover:bg-hover hover:text-text-primary"
        >
          <svg
            className="h-5 w-5"
            fill="none"
            stroke="currentColor"
            strokeWidth={1.8}
            viewBox="0 0 24 24"
          >
            <path strokeLinecap="round" d="M4 7h16M4 12h16M4 17h16" />
          </svg>
        </button>

        <h1 className="min-w-0 flex-1 truncate text-center text-base font-semibold tracking-tight">
          {projectName || t('common:app.nexusAI', 'GitNexus')}
        </h1>

        <button
          type="button"
          onClick={() => setSettingsPanelOpen(true)}
          aria-label={t('header:aiSettings', 'Settings')}
          className="-mr-1 flex h-9 w-9 items-center justify-center rounded-lg text-text-secondary transition-colors hover:bg-hover hover:text-text-primary"
        >
          <Settings className="h-5 w-5" />
        </button>
      </header>

      {/* Tabbed content — all panes stay mounted so sigma keeps real dimensions
          and chat scroll state survives a tab switch. */}
      <div className="relative min-h-0 flex-1">
        <div
          className={`absolute inset-0 ${activeTab === 'graph' ? '' : 'pointer-events-none invisible'}`}
          aria-hidden={activeTab !== 'graph'}
        >
          <div className="relative h-full w-full">
            <GraphCanvas ref={graphCanvasRef} />
            <UAToolbar />
            {isCodePanelOpen && (codeReferences.length > 0 || !!selectedNode) && (
              <div className="pointer-events-auto absolute inset-y-0 left-0 z-30">
                <CodeReferencesPanel onFocusNode={handleFocusNode} />
              </div>
            )}
          </div>
        </div>

        <div
          className={`absolute inset-0 overflow-auto bg-surface ${
            activeTab === 'files' ? '' : 'pointer-events-none invisible'
          }`}
          aria-hidden={activeTab !== 'files'}
        >
          <FileTreePanel onFocusNode={handleFocusNode} />
        </div>

        <div
          className={`absolute inset-0 ${activeTab === 'chat' ? '' : 'pointer-events-none invisible'}`}
          aria-hidden={activeTab !== 'chat'}
        >
          {isRightPanelOpen && <RightPanel />}
        </div>
      </div>

      {/* Bottom tab nav */}
      <MobileBottomNav activeTab={activeTab} onTabChange={setActiveTab} />

      {/* Slide-in drawer (secondary controls) */}
      <div
        className={`fixed inset-0 z-40 ${drawerOpen ? 'pointer-events-auto' : 'pointer-events-none'}`}
        aria-hidden={!drawerOpen}
      >
        <button
          type="button"
          aria-label={t('common:close', 'Close')}
          onClick={() => setDrawerOpen(false)}
          className={`absolute inset-0 bg-black/65 backdrop-blur-sm transition-opacity duration-300 ${
            drawerOpen ? 'opacity-100' : 'opacity-0'
          }`}
        />
        <aside
          className={`absolute top-0 bottom-0 left-0 flex w-[86%] max-w-[360px] flex-col border-r border-border-subtle bg-surface transition-transform duration-300 ease-out ${
            drawerOpen ? 'translate-x-0' : '-translate-x-full'
          }`}
          role="dialog"
        >
          <header className="flex shrink-0 items-center justify-between border-b border-border-subtle px-5 py-4">
            <div>
              <span className="text-[10px] font-semibold tracking-[0.2em] text-accent uppercase">
                {t('common:app.nexusAI', 'GitNexus')}
              </span>
              <h2 className="mt-0.5 text-lg leading-none font-semibold text-text-primary">
                {projectName || ''}
              </h2>
            </div>
            <button
              type="button"
              onClick={() => setDrawerOpen(false)}
              aria-label={t('common:close', 'Close')}
              className="flex h-9 w-9 items-center justify-center rounded-lg text-text-muted transition-colors hover:bg-elevated hover:text-text-primary"
            >
              <X className="h-5 w-5" />
            </button>
          </header>

          <div className="flex-1 space-y-6 overflow-auto px-5 py-5">
            {availableRepos.length > 0 && (
              <section>
                <h3 className="mb-3 text-[10px] font-semibold tracking-[0.18em] text-text-muted uppercase">
                  {t('header:repositories', 'Repositories')}
                </h3>
                <div className="space-y-1">
                  {availableRepos.map((repo) => {
                    const active = repo.name === projectName;
                    return (
                      <button
                        key={repo.name}
                        type="button"
                        onClick={() => {
                          onSwitchRepo(repo.name);
                          setDrawerOpen(false);
                        }}
                        className={`flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm transition-colors ${
                          active
                            ? 'bg-accent/15 text-accent'
                            : 'text-text-secondary hover:bg-elevated hover:text-text-primary'
                        }`}
                      >
                        <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-node-function" />
                        <span className="truncate">{repo.name}</span>
                      </button>
                    );
                  })}
                </div>
              </section>
            )}

            <section>
              <h3 className="mb-3 text-[10px] font-semibold tracking-[0.18em] text-text-muted uppercase">
                {t('common:preferences', 'Preferences')}
              </h3>
              <div className="flex flex-wrap items-center gap-2">
                <LanguageSwitcher />
                <ThemePicker />
              </div>
            </section>

            <section>
              <h3 className="mb-3 text-[10px] font-semibold tracking-[0.18em] text-text-muted uppercase">
                {t('header:help', 'Help')}
              </h3>
              <div className="flex flex-col gap-2">
                <button
                  type="button"
                  onClick={() => {
                    setSettingsPanelOpen(true);
                    setDrawerOpen(false);
                  }}
                  className="flex items-center gap-2 rounded-lg bg-elevated px-3 py-2 text-sm text-text-secondary transition-colors hover:text-text-primary"
                >
                  <Settings className="h-4 w-4" />
                  {t('header:aiSettings', 'Settings')}
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setHelpDialogBoxOpen(true);
                    setDrawerOpen(false);
                  }}
                  className="flex items-center gap-2 rounded-lg bg-elevated px-3 py-2 text-sm text-text-secondary transition-colors hover:text-text-primary"
                >
                  <HelpCircle className="h-4 w-4" />
                  {t('header:help', 'Help')}
                </button>
              </div>
            </section>
          </div>
        </aside>
      </div>

      {/* Settings modal (also reachable from the top bar) */}
      <SettingsPanel
        isOpen={isSettingsPanelOpen}
        onClose={() => setSettingsPanelOpen(false)}
        onSettingsSaved={handleSettingsSaved}
      />

      {/* Empty-state hint */}
      {!graph && (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center text-sm text-text-muted">
          {t('common:loading', 'Loading…')}
        </div>
      )}
    </div>
  );
}
