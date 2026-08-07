import { useEffect, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { useAppState } from '../hooks/useAppState';
import { useUAFeatures } from '../hooks/useUAFeatures';
import { getActiveSigma } from '../lib/sigma-registry';

/**
 * Guided tour panel — ported from Understand-Anything's LearnPanel.
 *
 * Tour steps are derived from GitNexus `Process` nodes (see `ua-adapter.ts`).
 * Adapted to GitNexus:
 *  - Drives sigma.js camera focus on each step's primary node (mirrors the
 *    `focusNode` logic in `useSigma`).
 *  - Highlights the step's referenced nodes via `useAppState`.
 *  - Renders only when `tourActive` (toggled from `useUAFeatures`).
 */
export default function TourPanel() {
  const { uaGraph, tourActive, tourStepIndex, endTour, nextTourStep, prevTourStep, goToTourStep } =
    useUAFeatures();
  const { graph, setSelectedNode, openCodePanel, setHighlightedNodeIds } = useAppState();
  const { t } = useTranslation(['common']);

  const steps = useMemo(
    () => (uaGraph?.tour ? [...uaGraph.tour].sort((a, b) => a.order - b.order) : []),
    [uaGraph?.tour],
  );

  const step = steps[tourStepIndex];

  // Highlight step nodes + focus camera whenever the active step changes.
  useEffect(() => {
    if (!tourActive || !step) {
      setHighlightedNodeIds(new Set());
      return;
    }
    setHighlightedNodeIds(new Set(step.nodeIds));

    const primary = step.nodeIds[0];
    if (primary) {
      const sigma = getActiveSigma();
      const sg = sigma?.getGraph();
      if (sigma && sg && sg.hasNode(primary)) {
        const attrs = sg.getNodeAttributes(primary) as { x: number; y: number };
        sigma.getCamera().animate({ x: attrs.x, y: attrs.y, ratio: 0.3 }, { duration: 400 });
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tourActive, tourStepIndex]);

  // Clear highlights when unmounting / tour ends.
  useEffect(() => {
    return () => setHighlightedNodeIds(new Set());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (!tourActive || !step) return null;

  const total = steps.length;
  const progressPct = ((tourStepIndex + 1) / total) * 100;
  const isFirst = tourStepIndex === 0;
  const isLast = tourStepIndex === total - 1;

  const selectNode = (id: string) => {
    const gn = graph?.nodes.find((n) => n.id === id);
    if (gn) {
      setSelectedNode(gn);
      openCodePanel();
    }
  };

  return (
    <div className="flex max-h-[70vh] w-[340px] flex-col overflow-hidden rounded-xl border border-border-default bg-elevated/95 shadow-2xl backdrop-blur-sm">
      {/* Header */}
      <div className="flex shrink-0 items-center justify-between border-b border-border-default px-3 py-2">
        <div className="flex items-center gap-2">
          <h3 className="text-[11px] font-semibold tracking-wider text-accent uppercase">
            {t('tour.title', 'Guided tour')}
          </h3>
          <span className="text-xs text-text-muted">
            {tourStepIndex + 1} / {total}
          </span>
        </div>
        <button
          onClick={endTour}
          className="text-[10px] text-text-muted transition-colors hover:text-text-secondary"
        >
          {t('tour.exit', 'Exit')}
        </button>
      </div>

      {/* Progress bar */}
      <div className="h-1 shrink-0 bg-surface">
        <div
          className="h-full bg-accent transition-all duration-300"
          style={{ width: `${progressPct}%` }}
        />
      </div>

      {/* Content */}
      <div className="min-h-0 flex-1 overflow-y-auto p-4">
        <h2 className="mb-3 text-lg font-semibold text-text-primary">{step.title}</h2>
        {step.description && (
          <p className="mb-4 text-sm leading-relaxed whitespace-pre-wrap text-text-secondary">
            {step.description}
          </p>
        )}

        {step.languageLesson && (
          <div className="mb-4 rounded border border-accent/20 bg-accent/5 p-3">
            <h4 className="mb-1.5 text-[11px] font-semibold tracking-wider text-accent uppercase">
              {t('tour.languageLesson', 'Language lesson')}
            </h4>
            <p className="text-sm leading-relaxed text-text-secondary">{step.languageLesson}</p>
          </div>
        )}

        {step.nodeIds.length > 0 && (
          <div className="mb-2">
            <h4 className="mb-2 text-[11px] font-semibold tracking-wider text-accent uppercase">
              {t('tour.referencedComponents', 'Referenced components')}
            </h4>
            <div className="flex flex-wrap gap-1.5">
              {step.nodeIds.map((nodeId) => {
                const node = uaGraph?.nodes.find((n) => n.id === nodeId);
                return (
                  <button
                    key={nodeId}
                    onClick={() => selectNode(nodeId)}
                    className="rounded-full border border-border-default bg-surface px-2.5 py-1 text-[11px] text-text-secondary transition-colors hover:text-text-primary"
                  >
                    {node?.name ?? nodeId}
                  </button>
                );
              })}
            </div>
          </div>
        )}
      </div>

      {/* Navigation */}
      <div className="shrink-0 border-t border-border-default px-3 py-2">
        <div className="mb-2 flex justify-center gap-1.5">
          {steps.map((_, i) => (
            <button
              key={i}
              onClick={() => goToTourStep(i)}
              className={`h-2 w-2 rounded-full transition-colors ${
                i === tourStepIndex ? 'bg-accent' : 'bg-surface hover:bg-hover'
              }`}
              aria-label={`Go to step ${i + 1}`}
            />
          ))}
        </div>
        <div className="flex gap-2">
          <button
            onClick={prevTourStep}
            disabled={isFirst}
            className="flex-1 rounded-lg bg-surface py-1.5 text-xs text-text-secondary transition-colors hover:bg-hover disabled:cursor-not-allowed disabled:opacity-40"
          >
            {t('tour.prev', 'Previous')}
          </button>
          <button
            onClick={isLast ? endTour : nextTourStep}
            className="flex-1 rounded-lg border border-accent/30 bg-accent/10 py-1.5 text-xs text-accent transition-colors hover:bg-accent/20"
          >
            {isLast ? t('tour.finish', 'Finish') : t('tour.next', 'Next')}
          </button>
        </div>
      </div>
    </div>
  );
}
