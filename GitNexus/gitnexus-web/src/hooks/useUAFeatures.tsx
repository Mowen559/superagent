import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { useAppState } from './useAppState';
import { adaptGnGraphToUA } from '../core/graph/ua-adapter';
import {
  UA_ALL_COMPLEXITIES,
  UA_ALL_EDGE_CATEGORIES,
  UA_ALL_NODE_TYPES,
  type UAComplexity,
  type UADomainGraph,
  type UAEdgeCategory,
  type UAGraphNode,
  type UAKnowledgeGraph,
  type UANodeType,
} from '../core/graph/ua-model';
import { generateDomainGraph } from '../core/graph/domain-graph-generator';
import { validateDomainGraph } from '../core/graph/domain-graph-schema';
import { fetchDomainGraph, saveDomainGraph } from '../services/backend-client';

/**
 * UA feature state — ported from Understand-Anything's Zustand store, but
 * implemented as a React Context to stay consistent with GitNexus' existing
 * state paradigm (useAppState) and avoid introducing a second state library.
 *
 * Holds:
 *  - `uaGraph`: GN graph adapted into UA's semantic model (memoised)
 *  - semantic filters (node type / complexity / layer / edge category)
 *  - layer navigation level (overview ↔ layer-detail)
 *  - guided tour playback state
 */

export type NavigationLevel = 'overview' | 'layer-detail';

/** Which graph the canvas is currently showing. */
export type GraphKind = 'structural' | 'domain';

/** Lifecycle of the (LLM-generated) domain graph. */
export type DomainGraphStatus = 'idle' | 'loading' | 'ready' | 'error';

export interface UAFilterState {
  nodeTypes: Set<UANodeType>;
  complexities: Set<UAComplexity>;
  layerIds: Set<string>;
  edgeCategories: Set<UAEdgeCategory>;
}

interface UAFeaturesValue {
  uaGraph: UAKnowledgeGraph | null;

  // Filters
  filters: UAFilterState;
  setFilters: (partial: Partial<UAFilterState>) => void;
  resetFilters: () => void;
  hasActiveFilters: () => boolean;
  filterPanelOpen: boolean;
  toggleFilterPanel: () => void;

  // Layer navigation
  navigationLevel: NavigationLevel;
  activeLayerId: string | null;
  drillIntoLayer: (layerId: string) => void;
  backToOverview: () => void;

  // Guided tour
  tourActive: boolean;
  tourStepIndex: number;
  startTour: () => void;
  endTour: () => void;
  nextTourStep: () => void;
  prevTourStep: () => void;
  goToTourStep: (index: number) => void;

  // Domain graph (LLM-generated, sigma-rendered)
  domainGraph: UADomainGraph | null;
  domainGraphStatus: DomainGraphStatus;
  domainGraphError: string | null;
  activeGraphKind: GraphKind;
  setActiveGraphKind: (kind: GraphKind) => void;
  generateDomainGraphAction: () => Promise<void>;
  cancelDomainGraphGeneration: () => void;
  selectedDomainNode: UAGraphNode | null;
  setSelectedDomainNode: (node: UAGraphNode | null) => void;
}

function defaultFilters(): UAFilterState {
  return {
    nodeTypes: new Set<UANodeType>(UA_ALL_NODE_TYPES),
    complexities: new Set<UAComplexity>(UA_ALL_COMPLEXITIES),
    layerIds: new Set<string>(),
    edgeCategories: new Set<UAEdgeCategory>(UA_ALL_EDGE_CATEGORIES),
  };
}

const UAFeaturesContext = createContext<UAFeaturesValue | null>(null);

export function UAFeaturesProvider({ children }: { children: ReactNode }) {
  const { graph, projectName } = useAppState();

  // Derive the UA semantic graph from GN's native graph. Recomputed only when
  // the underlying node/relationship counts change (cheap identity proxy).
  const uaGraph = useMemo<UAKnowledgeGraph | null>(() => {
    if (!graph) return null;
    return adaptGnGraphToUA(graph.nodes, graph.relationships);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [graph, graph?.nodeCount, graph?.relationshipCount]);

  const [filters, setFiltersState] = useState<UAFilterState>(defaultFilters);
  const [filterPanelOpen, setFilterPanelOpen] = useState(false);
  const [navigationLevel, setNavigationLevel] = useState<NavigationLevel>('overview');
  const [activeLayerId, setActiveLayerId] = useState<string | null>(null);
  const [tourActive, setTourActive] = useState(false);
  const [tourStepIndex, setTourStepIndex] = useState(0);

  const setFilters = useCallback((partial: Partial<UAFilterState>) => {
    setFiltersState((prev) => ({ ...prev, ...partial }));
  }, []);

  const resetFilters = useCallback(() => {
    setFiltersState(defaultFilters());
  }, []);

  const hasActiveFilters = useCallback(() => {
    return (
      filters.nodeTypes.size !== UA_ALL_NODE_TYPES.length ||
      filters.complexities.size !== UA_ALL_COMPLEXITIES.length ||
      filters.layerIds.size !== 0 ||
      filters.edgeCategories.size !== UA_ALL_EDGE_CATEGORIES.length
    );
  }, [filters]);

  const toggleFilterPanel = useCallback(() => {
    setFilterPanelOpen((v) => !v);
  }, []);

  const drillIntoLayer = useCallback((layerId: string) => {
    setActiveLayerId(layerId);
    setNavigationLevel('layer-detail');
  }, []);

  const backToOverview = useCallback(() => {
    setActiveLayerId(null);
    setNavigationLevel('overview');
  }, []);

  const tourLength = uaGraph?.tour.length ?? 0;

  const startTour = useCallback(() => {
    if (tourLength === 0) return;
    setTourStepIndex(0);
    setTourActive(true);
  }, [tourLength]);

  const endTour = useCallback(() => {
    setTourActive(false);
  }, []);

  const nextTourStep = useCallback(() => {
    setTourStepIndex((i) => Math.min(i + 1, Math.max(0, tourLength - 1)));
  }, [tourLength]);

  const prevTourStep = useCallback(() => {
    setTourStepIndex((i) => Math.max(0, i - 1));
  }, []);

  const goToTourStep = useCallback(
    (index: number) => {
      setTourStepIndex(Math.min(Math.max(0, index), Math.max(0, tourLength - 1)));
    },
    [tourLength],
  );

  // ── Domain graph (LLM-generated, sigma-rendered) ─────────────────────────
  const [domainGraph, setDomainGraph] = useState<UADomainGraph | null>(null);
  const [domainGraphStatus, setDomainGraphStatus] = useState<DomainGraphStatus>('idle');
  const [domainGraphError, setDomainGraphError] = useState<string | null>(null);
  const [activeGraphKind, setActiveGraphKindState] = useState<GraphKind>('structural');
  const [selectedDomainNode, setSelectedDomainNode] = useState<UAGraphNode | null>(null);
  const domainAbortRef = useRef<AbortController | null>(null);

  // Reset domain state on repo change, then best-effort load any cached graph.
  useEffect(() => {
    domainAbortRef.current?.abort();
    setDomainGraph(null);
    setDomainGraphStatus('idle');
    setDomainGraphError(null);
    setActiveGraphKindState('structural');
    setSelectedDomainNode(null);

    if (!projectName) return;
    let cancelled = false;
    void (async () => {
      try {
        const cached = await fetchDomainGraph(projectName);
        if (cancelled || !cached) return;
        const result = validateDomainGraph(cached);
        if (result.ok && result.data) {
          setDomainGraph(result.data);
          setDomainGraphStatus('ready');
        }
      } catch {
        // Cache is best-effort; ignore load failures.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [projectName]);

  const setActiveGraphKind = useCallback((kind: GraphKind) => {
    setActiveGraphKindState(kind);
  }, []);

  const generateDomainGraphAction = useCallback(async () => {
    if (!uaGraph) return;
    domainAbortRef.current?.abort();
    const controller = new AbortController();
    domainAbortRef.current = controller;
    setDomainGraphStatus('loading');
    setDomainGraphError(null);
    try {
      const result = await generateDomainGraph(uaGraph, controller.signal);
      if (controller.signal.aborted) return;
      setDomainGraph(result);
      setDomainGraphStatus('ready');
      // Persist is best-effort — don't fail the UI if the backend rejects it.
      try {
        await saveDomainGraph(projectName || undefined, result);
      } catch {
        /* ignore persistence errors */
      }
    } catch (err) {
      if (controller.signal.aborted) return;
      setDomainGraphStatus('error');
      setDomainGraphError(err instanceof Error ? err.message : String(err));
    }
  }, [uaGraph, projectName]);

  const cancelDomainGraphGeneration = useCallback(() => {
    domainAbortRef.current?.abort();
    setDomainGraphStatus((s) => (s === 'loading' ? 'idle' : s));
  }, []);

  const value = useMemo<UAFeaturesValue>(
    () => ({
      uaGraph,
      filters,
      setFilters,
      resetFilters,
      hasActiveFilters,
      filterPanelOpen,
      toggleFilterPanel,
      navigationLevel,
      activeLayerId,
      drillIntoLayer,
      backToOverview,
      tourActive,
      tourStepIndex,
      startTour,
      endTour,
      nextTourStep,
      prevTourStep,
      goToTourStep,
      domainGraph,
      domainGraphStatus,
      domainGraphError,
      activeGraphKind,
      setActiveGraphKind,
      generateDomainGraphAction,
      cancelDomainGraphGeneration,
      selectedDomainNode,
      setSelectedDomainNode,
    }),
    [
      uaGraph,
      filters,
      setFilters,
      resetFilters,
      hasActiveFilters,
      filterPanelOpen,
      toggleFilterPanel,
      navigationLevel,
      activeLayerId,
      drillIntoLayer,
      backToOverview,
      tourActive,
      tourStepIndex,
      startTour,
      endTour,
      nextTourStep,
      prevTourStep,
      goToTourStep,
      domainGraph,
      domainGraphStatus,
      domainGraphError,
      activeGraphKind,
      setActiveGraphKind,
      generateDomainGraphAction,
      cancelDomainGraphGeneration,
      selectedDomainNode,
    ],
  );

  return <UAFeaturesContext.Provider value={value}>{children}</UAFeaturesContext.Provider>;
}

export function useUAFeatures(): UAFeaturesValue {
  const ctx = useContext(UAFeaturesContext);
  if (!ctx) throw new Error('useUAFeatures must be used within UAFeaturesProvider');
  return ctx;
}
