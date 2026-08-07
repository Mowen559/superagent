import { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { useUAFeatures } from '../hooks/useUAFeatures';
import {
  UA_ALL_NODE_TYPES,
  UA_ALL_COMPLEXITIES,
  UA_ALL_EDGE_CATEGORIES,
  type UANodeType,
  type UAComplexity,
  type UAEdgeCategory,
} from '../core/graph/ua-model';

/**
 * Semantic filter dropdown — ported from Understand-Anything dashboard.
 * Adapted to GitNexus: consumes `useUAFeatures` (React Context) instead of
 * the UA Zustand store, react-i18next instead of UA's I18nContext, and GN's
 * `accent` theme token instead of UA's `gold`.
 */
export default function FilterPanel() {
  const {
    uaGraph,
    filters,
    setFilters,
    resetFilters,
    hasActiveFilters,
    filterPanelOpen,
    toggleFilterPanel,
  } = useUAFeatures();
  const { t } = useTranslation(['common']);

  const containerRef = useRef<HTMLDivElement>(null);

  const layers = uaGraph?.layers ?? [];

  // Close dropdown on outside click
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        if (filterPanelOpen) {
          toggleFilterPanel();
        }
      }
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, [filterPanelOpen, toggleFilterPanel]);

  const toggleNodeType = (type: UANodeType) => {
    const newTypes = new Set(filters.nodeTypes);
    if (newTypes.has(type)) newTypes.delete(type);
    else newTypes.add(type);
    setFilters({ nodeTypes: newTypes });
  };

  const toggleComplexity = (complexity: UAComplexity) => {
    const newComplexities = new Set(filters.complexities);
    if (newComplexities.has(complexity)) newComplexities.delete(complexity);
    else newComplexities.add(complexity);
    setFilters({ complexities: newComplexities });
  };

  const toggleLayer = (layerId: string) => {
    const newLayers = new Set(filters.layerIds);
    if (newLayers.has(layerId)) newLayers.delete(layerId);
    else newLayers.add(layerId);
    setFilters({ layerIds: newLayers });
  };

  const toggleEdgeCategory = (category: UAEdgeCategory) => {
    const newCategories = new Set(filters.edgeCategories);
    if (newCategories.has(category)) newCategories.delete(category);
    else newCategories.add(category);
    setFilters({ edgeCategories: newCategories });
  };

  const isActive = hasActiveFilters();

  return (
    <div ref={containerRef} className="relative">
      <button
        onClick={toggleFilterPanel}
        className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm transition-colors ${
          isActive
            ? 'bg-accent/20 text-accent hover:bg-accent/30'
            : 'bg-elevated text-text-secondary hover:text-text-primary'
        }`}
        title={t('filter.title', 'Filter graph')}
      >
        <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M3 4a1 1 0 011-1h16a1 1 0 011 1v2.586a1 1 0 01-.293.707l-6.414 6.414a1 1 0 00-.293.707V17l-4 4v-6.586a1 1 0 00-.293-.707L3.293 7.293A1 1 0 013 6.586V4z"
          />
        </svg>
        {t('filter.filter', 'Filter')}
      </button>

      {filterPanelOpen && (
        <div className="absolute top-full right-0 z-50 mt-2 max-h-[70vh] w-72 animate-fade-in scrollbar-thin overflow-hidden overflow-y-auto rounded-lg border border-border-default bg-elevated shadow-xl">
          <div className="space-y-4 p-4">
            {/* Node Types */}
            <div>
              <h3 className="mb-2 text-xs font-semibold tracking-wider text-text-secondary uppercase">
                {t('filter.nodeTypes', 'Node types')}
              </h3>
              <div className="space-y-1.5">
                {UA_ALL_NODE_TYPES.map((type) => (
                  <label
                    key={type}
                    className="flex cursor-pointer items-center gap-2 rounded px-2 py-1 transition-colors hover:bg-hover/50"
                  >
                    <input
                      type="checkbox"
                      checked={filters.nodeTypes.has(type)}
                      onChange={() => toggleNodeType(type)}
                      className="h-3.5 w-3.5 cursor-pointer rounded border-border-subtle bg-elevated accent-[var(--color-accent)]"
                    />
                    <span className="text-sm text-text-primary capitalize">{type}</span>
                  </label>
                ))}
              </div>
            </div>

            {/* Complexity */}
            <div>
              <h3 className="mb-2 text-xs font-semibold tracking-wider text-text-secondary uppercase">
                {t('filter.complexity', 'Complexity')}
              </h3>
              <div className="space-y-1.5">
                {UA_ALL_COMPLEXITIES.map((complexity) => (
                  <label
                    key={complexity}
                    className="flex cursor-pointer items-center gap-2 rounded px-2 py-1 transition-colors hover:bg-hover/50"
                  >
                    <input
                      type="checkbox"
                      checked={filters.complexities.has(complexity)}
                      onChange={() => toggleComplexity(complexity)}
                      className="h-3.5 w-3.5 cursor-pointer rounded border-border-subtle bg-elevated accent-[var(--color-accent)]"
                    />
                    <span className="text-sm text-text-primary capitalize">{complexity}</span>
                  </label>
                ))}
              </div>
            </div>

            {/* Layers */}
            {layers.length > 0 && (
              <div>
                <h3 className="mb-2 text-xs font-semibold tracking-wider text-text-secondary uppercase">
                  {t('filter.layers', 'Layers')}
                </h3>
                <div className="space-y-1.5">
                  {layers.map((layer) => (
                    <label
                      key={layer.id}
                      className="flex cursor-pointer items-center gap-2 rounded px-2 py-1 transition-colors hover:bg-hover/50"
                    >
                      <input
                        type="checkbox"
                        checked={filters.layerIds.has(layer.id)}
                        onChange={() => toggleLayer(layer.id)}
                        className="h-3.5 w-3.5 cursor-pointer rounded border-border-subtle bg-elevated accent-[var(--color-accent)]"
                      />
                      <div className="h-2 w-2 shrink-0 rounded-full bg-accent/50" />
                      <span className="text-sm text-text-primary">{layer.name}</span>
                    </label>
                  ))}
                </div>
              </div>
            )}

            {/* Edge Categories */}
            <div>
              <h3 className="mb-2 text-xs font-semibold tracking-wider text-text-secondary uppercase">
                {t('filter.edgeCategories', 'Edge categories')}
              </h3>
              <div className="space-y-1.5">
                {UA_ALL_EDGE_CATEGORIES.map((category) => (
                  <label
                    key={category}
                    className="flex cursor-pointer items-center gap-2 rounded px-2 py-1 transition-colors hover:bg-hover/50"
                  >
                    <input
                      type="checkbox"
                      checked={filters.edgeCategories.has(category)}
                      onChange={() => toggleEdgeCategory(category)}
                      className="h-3.5 w-3.5 cursor-pointer rounded border-border-subtle bg-elevated accent-[var(--color-accent)]"
                    />
                    <span className="text-sm text-text-primary capitalize">
                      {category.replace(/-/g, ' ')}
                    </span>
                  </label>
                ))}
              </div>
            </div>

            {/* Reset Button */}
            {isActive && (
              <button
                onClick={resetFilters}
                className="w-full rounded-lg bg-elevated px-3 py-1.5 text-sm text-text-secondary transition-colors hover:bg-accent/20 hover:text-accent"
              >
                {t('filter.resetAll', 'Reset all')}
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
