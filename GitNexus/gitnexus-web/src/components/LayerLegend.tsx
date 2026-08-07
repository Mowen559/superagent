import { useTranslation } from 'react-i18next';
import { useUAFeatures } from '../hooks/useUAFeatures';

/**
 * Layer legend / cluster navigator — ported from Understand-Anything.
 *
 * Layers are derived from GitNexus `Community` nodes (see `ua-adapter.ts`).
 * In overview mode it lists all layers with a colour swatch and node count;
 * clicking a layer drills into it (filtering the sigma graph to that cluster).
 * In layer-detail mode it shows the active layer with a back affordance.
 */

// Shared layer colour palette (mirrors UA's LAYER_PALETTE).
export const LAYER_PALETTE = [
  { bg: 'rgba(59, 130, 246, 0.12)', border: 'rgba(59, 130, 246, 0.4)', label: '#3b82f6' }, // blue
  { bg: 'rgba(16, 185, 129, 0.12)', border: 'rgba(16, 185, 129, 0.4)', label: '#10b981' }, // green
  { bg: 'rgba(124, 58, 237, 0.12)', border: 'rgba(124, 58, 237, 0.4)', label: '#7c3aed' }, // violet
  { bg: 'rgba(245, 158, 11, 0.12)', border: 'rgba(245, 158, 11, 0.4)', label: '#f59e0b' }, // amber
  { bg: 'rgba(236, 72, 153, 0.12)', border: 'rgba(236, 72, 153, 0.4)', label: '#ec4899' }, // pink
  { bg: 'rgba(20, 184, 166, 0.12)', border: 'rgba(20, 184, 166, 0.4)', label: '#14b8a6' }, // teal
  { bg: 'rgba(120, 130, 145, 0.12)', border: 'rgba(120, 130, 145, 0.4)', label: '#788291' }, // slate
];

export function getLayerColor(index: number) {
  return LAYER_PALETTE[index % LAYER_PALETTE.length];
}

export default function LayerLegend() {
  const { uaGraph, navigationLevel, activeLayerId, drillIntoLayer, backToOverview } =
    useUAFeatures();
  const { t } = useTranslation(['common']);

  const layers = uaGraph?.layers ?? [];
  if (layers.length === 0) return null;

  const activeLayer = layers.find((l) => l.id === activeLayerId);

  return (
    <div className="flex items-center gap-2 rounded-lg border border-border-default bg-elevated/80 px-3 py-1.5 backdrop-blur-sm">
      {navigationLevel === 'layer-detail' ? (
        <button
          onClick={backToOverview}
          className="flex items-center gap-1 text-[11px] font-semibold whitespace-nowrap text-accent transition-opacity hover:opacity-80"
        >
          <span>←</span>
          <span>{t('layer.back', 'All layers')}</span>
        </button>
      ) : (
        <span className="text-[11px] font-medium whitespace-nowrap text-text-secondary">
          {layers.length} {t('layer.label', 'layers')}
        </span>
      )}

      <span className="text-[11px] text-text-muted">│</span>

      <div className="flex items-center gap-3 overflow-x-auto">
        {layers.map((layer, i) => {
          const color = getLayerColor(i);
          const isActive = navigationLevel === 'layer-detail' && layer.id === activeLayerId;
          const dimmed = navigationLevel === 'layer-detail' && !isActive;
          return (
            <button
              key={layer.id}
              onClick={() => drillIntoLayer(layer.id)}
              className="flex items-center gap-1 whitespace-nowrap transition-opacity hover:opacity-100"
              style={{ opacity: dimmed ? 0.4 : 1 }}
              title={layer.description || layer.name}
            >
              <span
                className="inline-block h-2 w-2 rounded-full"
                style={{ backgroundColor: color.label }}
              />
              <span
                className={`text-[11px] ${isActive ? 'font-medium text-text-primary' : 'text-text-secondary'}`}
              >
                {layer.name}
                <span className="ml-0.5 text-text-muted">({layer.nodeIds.length})</span>
              </span>
            </button>
          );
        })}
      </div>

      {activeLayer && (
        <span className="ml-1 text-[11px] whitespace-nowrap text-text-muted">
          · {activeLayer.nodeIds.length} {t('layer.nodes', 'nodes')}
        </span>
      )}
    </div>
  );
}
