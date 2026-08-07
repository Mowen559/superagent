import type Sigma from 'sigma';

/**
 * Lightweight module-level registry for the active Sigma instance.
 *
 * Sigma is created inside the `useSigma` hook and held in a private ref. Some
 * cross-cutting features ported from Understand-Anything (image/SVG export,
 * camera focus for guided tours and layer drill-down) need access to the live
 * instance without threading it through the whole component tree.
 *
 * GraphCanvas registers the instance once it is created; consumers read it via
 * `getActiveSigma()`. Only one canvas is mounted at a time, so a single slot
 * is sufficient.
 */
let activeSigma: Sigma | null = null;

export function setActiveSigma(sigma: Sigma | null): void {
  activeSigma = sigma;
}

export function getActiveSigma(): Sigma | null {
  return activeSigma;
}
