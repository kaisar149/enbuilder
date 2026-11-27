// The one place that decides which 3D renderer is used. Three.js is imported lazily so the 2D-only bundle does not carry it.
// To swap the renderer: change `createViewport3D` (a stub is built in as the fallback and as the test double).
import type { Viewport3D } from './types';

export type Renderer3D = 'three' | 'stub';

function webglAvailable(): boolean {
  try {
    const c = document.createElement('canvas');
    return !!(c.getContext('webgl2') || c.getContext('webgl'));
  } catch { return false; }
}

export async function createViewport3D(host: HTMLElement, renderer: Renderer3D = 'three'): Promise<{ viewport: Viewport3D; renderer: Renderer3D }> {
  if (renderer === 'three' && webglAvailable()) {
    const { MeshViewport3D } = await import('./MeshViewport3D');
    return { viewport: new MeshViewport3D(host), renderer: 'three' };
  }
  const { StubViewport } = await import('./StubViewport');
  return { viewport: new StubViewport(host), renderer: 'stub' };
}
