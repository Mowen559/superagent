import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { getActiveSigma } from '../lib/sigma-registry';
import { useUAFeatures } from '../hooks/useUAFeatures';

/**
 * Export menu — ported from Understand-Anything dashboard.
 *
 * Adapted to GitNexus' sigma.js renderer (UA used React Flow):
 *  - PNG: composite the sigma canvas layers onto one canvas and download.
 *  - SVG: build a vector snapshot from the live graphology node positions
 *    projected to screen space via the sigma camera.
 *  - JSON: export the UA-adapted semantic graph.
 */

function escapeXml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function downloadBlob(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

export default function ExportMenu() {
  const { uaGraph } = useUAFeatures();
  const { t } = useTranslation(['common']);
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  const projectName = uaGraph?.project.name ?? 'knowledge-graph';

  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, []);

  const exportPNG = () => {
    const sigma = getActiveSigma();
    if (!sigma) {
      alert('Graph not ready for export');
      return;
    }
    try {
      const container = sigma.getContainer();
      const canvases = Array.from(container.querySelectorAll('canvas'));
      if (canvases.length === 0) {
        alert('No graph to export');
        return;
      }
      const { width, height } = canvases[0];
      const out = document.createElement('canvas');
      out.width = width;
      out.height = height;
      const ctx = out.getContext('2d');
      if (!ctx) {
        alert('Failed to create canvas context');
        return;
      }
      // Opaque background matching the app surface.
      ctx.fillStyle =
        getComputedStyle(document.documentElement).getPropertyValue('--color-void').trim() ||
        '#0a0a0a';
      ctx.fillRect(0, 0, width, height);
      for (const c of canvases) {
        ctx.drawImage(c, 0, 0, width, height);
      }
      out.toBlob((blob) => {
        if (blob) {
          downloadBlob(blob, `${projectName}-export.png`);
          setOpen(false);
        } else {
          alert('Failed to export PNG: image encoding failed.');
        }
      }, 'image/png');
    } catch (error) {
      console.error('PNG export failed:', error);
      alert(`Failed to export PNG: ${error instanceof Error ? error.message : String(error)}`);
    }
  };

  const exportSVG = () => {
    const sigma = getActiveSigma();
    if (!sigma) {
      alert('Graph not ready for export');
      return;
    }
    try {
      const graph = sigma.getGraph();
      if (graph.order === 0) {
        alert('No nodes to export');
        return;
      }
      const { width, height } = sigma.getDimensions();
      const bg =
        getComputedStyle(document.documentElement).getPropertyValue('--color-void').trim() ||
        '#0a0a0a';

      let svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">`;
      svg += `<rect width="100%" height="100%" fill="${bg}"/>`;

      // Edges (project both endpoints to screen space).
      graph.forEachEdge((_e, _attr, source, target) => {
        const s = sigma.graphToViewport(
          graph.getNodeAttributes(source) as { x: number; y: number },
        );
        const tp = sigma.graphToViewport(
          graph.getNodeAttributes(target) as { x: number; y: number },
        );
        svg += `<line x1="${s.x.toFixed(1)}" y1="${s.y.toFixed(1)}" x2="${tp.x.toFixed(1)}" y2="${tp.y.toFixed(1)}" stroke="rgba(124,58,237,0.3)" stroke-width="1"/>`;
      });

      // Nodes.
      graph.forEachNode((_n, attr) => {
        const p = sigma.graphToViewport(attr as { x: number; y: number });
        const color = (attr.color as string) ?? '#7c3aed';
        const r = Math.max(3, (attr.size as number) ?? 5);
        svg += `<circle cx="${p.x.toFixed(1)}" cy="${p.y.toFixed(1)}" r="${r.toFixed(1)}" fill="${escapeXml(color)}"/>`;
        const label = attr.label as string | undefined;
        if (label) {
          svg += `<text x="${(p.x + r + 2).toFixed(1)}" y="${(p.y + 3).toFixed(1)}" fill="#e4e4ed" font-size="10">${escapeXml(label)}</text>`;
        }
      });

      svg += `</svg>`;
      downloadBlob(
        new Blob([svg], { type: 'image/svg+xml;charset=utf-8' }),
        `${projectName}-export.svg`,
      );
      setOpen(false);
    } catch (error) {
      console.error('SVG export failed:', error);
      alert(`Failed to export SVG: ${error instanceof Error ? error.message : String(error)}`);
    }
  };

  const exportJSON = () => {
    if (!uaGraph) {
      alert('No graph loaded');
      return;
    }
    try {
      const json = JSON.stringify(uaGraph, null, 2);
      downloadBlob(new Blob([json], { type: 'application/json' }), `${projectName}-export.json`);
      setOpen(false);
    } catch (error) {
      console.error('JSON export failed:', error);
      alert(`Failed to export JSON: ${error instanceof Error ? error.message : String(error)}`);
    }
  };

  return (
    <div ref={containerRef} className="relative">
      <button
        onClick={() => setOpen((v) => !v)}
        className="flex items-center gap-1.5 rounded-lg bg-elevated px-3 py-1.5 text-sm text-text-secondary transition-colors hover:text-text-primary"
        title={t('export.title', 'Export graph')}
      >
        <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4"
          />
        </svg>
        {t('export.label', 'Export')}
      </button>

      {open && (
        <div className="absolute top-full right-0 z-50 mt-2 w-52 animate-fade-in overflow-hidden rounded-lg border border-border-default bg-elevated shadow-xl">
          <div className="p-2">
            <button
              onClick={exportPNG}
              className="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-sm text-text-primary transition-colors hover:bg-hover"
            >
              <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M4 16l4.586-4.586a2 2 0 012.828 0L16 16m-2-2l1.586-1.586a2 2 0 012.828 0L20 14m-6-6h.01M6 20h12a2 2 0 002-2V6a2 2 0 00-2-2H6a2 2 0 00-2 2v12a2 2 0 002 2z"
                />
              </svg>
              <span>{t('export.asPNG', 'Export as PNG')}</span>
            </button>
            <button
              onClick={exportSVG}
              className="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-sm text-text-primary transition-colors hover:bg-hover"
            >
              <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M7 21a4 4 0 01-4-4V5a2 2 0 012-2h4a2 2 0 012 2v12a4 4 0 01-4 4zm0 0h12a2 2 0 002-2v-4a2 2 0 00-2-2h-2.343M11 7.343l1.657-1.657a2 2 0 012.828 0l2.829 2.829a2 2 0 010 2.828l-8.486 8.485M7 17h.01"
                />
              </svg>
              <span>{t('export.asSVG', 'Export as SVG')}</span>
            </button>
            <button
              onClick={exportJSON}
              disabled={!uaGraph}
              className="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-sm text-text-primary transition-colors hover:bg-hover disabled:cursor-not-allowed disabled:opacity-50"
            >
              <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4"
                />
              </svg>
              <span>{t('export.asJSON', 'Export as JSON')}</span>
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
