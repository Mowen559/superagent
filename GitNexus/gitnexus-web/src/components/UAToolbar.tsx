import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useUAFeatures } from '../hooks/useUAFeatures';
import FilterPanel from './FilterPanel';
import ExportMenu from './ExportMenu';
import PathFinderModal from './PathFinderModal';
import NodeInfo from './NodeInfo';
import TourPanel from './TourPanel';

/**
 * Unified UA feature toolbar — the single UI entry point that wires the ported
 * Understand-Anything tools into GitNexus' graph view.
 *
 * Layout (overlaid on the graph canvas, top-right):
 *  - Filter  → FilterPanel (self-contained button + dropdown)
 *  - Export  → ExportMenu  (self-contained button + dropdown)
 *  - Path    → toggles PathFinderModal
 *  - Details → toggles the NodeInfo semantic detail panel
 *  - Tour    → starts the guided tour (renders TourPanel while active)
 *
 * Mounted once inside the graph area of App.tsx.
 */
export default function UAToolbar() {
  const {
    uaGraph,
    startTour,
    tourActive,
    activeGraphKind,
    setActiveGraphKind,
    domainGraph,
    domainGraphStatus,
    generateDomainGraphAction,
  } = useUAFeatures();
  const { t } = useTranslation(['common']);

  const [pathFinderOpen, setPathFinderOpen] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);

  // Hide the toolbar until a graph is loaded.
  if (!uaGraph) return null;

  const hasTour = uaGraph.tour.length > 0;
  const isDomainView = activeGraphKind === 'domain';

  const switchToDomain = () => {
    setActiveGraphKind('domain');
    // Kick off generation on first switch when nothing is cached/loaded yet.
    if (!domainGraph && domainGraphStatus === 'idle') {
      void generateDomainGraphAction();
    }
  };

  const buttonClass =
    'flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-sm bg-elevated text-text-secondary hover:text-text-primary transition-colors';

  return (
    <>
      {/* Toolbar */}
      <div className="pointer-events-auto absolute top-4 right-4 z-20 flex items-center gap-2">
        {/* Structural / Domain graph switch */}
        <div className="flex items-center rounded-lg border border-border-subtle bg-elevated/90 p-0.5">
          <button
            onClick={() => setActiveGraphKind('structural')}
            className={`rounded-md px-2.5 py-1 text-xs font-medium transition-colors ${
              !isDomainView ? 'bg-accent text-white' : 'text-text-secondary hover:text-text-primary'
            }`}
            title={t('domainGraph.structuralTab', 'Structure')}
          >
            {t('domainGraph.structuralTab', 'Structure')}
          </button>
          <button
            onClick={switchToDomain}
            className={`rounded-md px-2.5 py-1 text-xs font-medium transition-colors ${
              isDomainView ? 'bg-accent text-white' : 'text-text-secondary hover:text-text-primary'
            }`}
            title={t('domainGraph.domainTab', 'Domain')}
          >
            {t('domainGraph.domainTab', 'Domain')}
          </button>
        </div>

        <FilterPanel />
        <ExportMenu />

        <button
          onClick={() => setPathFinderOpen(true)}
          className={buttonClass}
          title={t('pathFinder.tooltip', 'Find dependency path')}
        >
          <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M13 7l5 5m0 0l-5 5m5-5H6"
            />
          </svg>
          {t('pathFinder.path', 'Path')}
        </button>

        <button
          onClick={() => setDetailsOpen((v) => !v)}
          className={`${
            detailsOpen
              ? 'flex items-center gap-1.5 rounded-lg bg-accent/20 px-3 py-1.5 text-sm text-accent transition-colors hover:bg-accent/30'
              : buttonClass
          }`}
          title={t('nodeInfo.title', 'Node details')}
        >
          <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
            />
          </svg>
          {t('nodeInfo.details', 'Details')}
        </button>

        {hasTour && (
          <button
            onClick={startTour}
            className={buttonClass}
            title={t('tour.start', 'Start guided tour')}
          >
            <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M14.752 11.168l-3.197-2.132A1 1 0 0010 9.87v4.263a1 1 0 001.555.832l3.197-2.132a1 1 0 000-1.664z"
              />
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
              />
            </svg>
            {t('tour.tour', 'Tour')}
          </button>
        )}
      </div>

      {/* Node detail panel (floating, right side under the toolbar) */}
      {detailsOpen && (
        <div className="pointer-events-auto absolute top-16 right-4 z-20 max-h-[calc(100vh-10rem)] w-[340px] overflow-hidden rounded-xl border border-border-default bg-elevated/95 shadow-2xl backdrop-blur-sm">
          <div className="flex items-center justify-between border-b border-border-default px-3 py-2">
            <h3 className="text-[11px] font-semibold tracking-wider text-accent uppercase">
              {t('nodeInfo.title', 'Node details')}
            </h3>
            <button
              onClick={() => setDetailsOpen(false)}
              className="text-[10px] text-text-muted transition-colors hover:text-text-secondary"
            >
              {t('common:close', 'Close')}
            </button>
          </div>
          <div className="max-h-[calc(100vh-13rem)] overflow-y-auto">
            <NodeInfo />
          </div>
        </div>
      )}

      {/* Guided tour panel (floating, bottom-right; renders only while active) */}
      {tourActive && (
        <div className="pointer-events-auto absolute right-4 bottom-4 z-20">
          <TourPanel />
        </div>
      )}

      {/* Path finder modal (full overlay) */}
      <PathFinderModal isOpen={pathFinderOpen} onClose={() => setPathFinderOpen(false)} />
    </>
  );
}
