// StubViewport: a 3D viewport that draws nothing. It proves the renderer is swappable in one place (factory.ts): input, camera, picking, hover,
// selection and colours all come from BaseViewport3D and keep working, only the pixels are missing. Also used in tests and as the fallback when
// WebGL is not available.
import { BaseViewport3D } from './BaseViewport3D';

export class StubViewport extends BaseViewport3D {
  /** What the renderer "received", for tests. */
  readonly log = { geometry: 0, positions: 0, colors: 0, view: 0, draws: 0, resizes: 0 };
  protected onGeometry(): void { this.log.geometry++; }
  protected onPositions(): void { this.log.positions++; }
  protected onColors(): void { this.log.colors++; }
  protected onView(): void { this.log.view++; }
  protected onSelection(): void { /* nothing to draw */ }
  protected onResize(): void { this.log.resizes++; }
  protected draw(): void { this.log.draws++; }
  protected onDispose(): void { /* nothing to free */ }
  /** Current per-cell colours (RGB triples) as the renderer would see them. */
  cellColours(): Uint8ClampedArray | null { return this.cellRGB; }
}
