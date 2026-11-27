// MeshViewport3D: the Three.js renderer. The ONLY file in the project that imports `three`. Everything else (input, camera, picking, colours,
// relief) is in BaseViewport3D; this class only uploads buffers and draws. Replace this file (see StubViewport.ts) to change renderer.
//
// Mesh -> triangles: geometry3d.buildCellGeometry makes one polygon per cell (corners = circumcentres of the cell and consecutive neighbours),
// triangulated as a fan. Corners are owned by one cell, so a cell has one flat colour and its own normal. Colours go in as sRGB bytes and the
// renderer works in linear-output mode with colour management off, so a cell has exactly the 2D map colour (parity test).
import * as THREE from 'three';
import { BaseViewport3D } from './BaseViewport3D';
import { cameraFrame, clipPlanes, FOV_DEG } from './orbit';
import { cornerColors } from './geometry3d';
import type { Overlay3D } from './overlay3d';

THREE.ColorManagement.enabled = false;

export class MeshViewport3D extends BaseViewport3D {
  private renderer: THREE.WebGLRenderer;
  private scene = new THREE.Scene();
  private cam = new THREE.PerspectiveCamera(FOV_DEG, 1, 0.01, 100);
  private geo = new THREE.BufferGeometry();
  private basic = new THREE.MeshBasicMaterial({ vertexColors: true });
  private lambert = new THREE.MeshLambertMaterial({ vertexColors: true });
  private meshObj: THREE.Mesh;
  private sun = new THREE.DirectionalLight(0xffffff, 1);
  private ambient = new THREE.AmbientLight(0xffffff, 0.5);
  private outline = new THREE.LineLoop(new THREE.BufferGeometry(), new THREE.LineBasicMaterial({ depthTest: false }));
  private colorAttr: Uint8Array | null = null;
  /** Part 9e: one object per overlay item, bottom to top. */
  private overlayGroup = new THREE.Group();

  constructor(host: HTMLElement) {
    super(host);
    this.renderer = new THREE.WebGLRenderer({ canvas: this.canvas, antialias: true, alpha: true, powerPreference: 'high-performance' });
    this.renderer.outputColorSpace = THREE.LinearSRGBColorSpace;
    this.renderer.setClearColor(0x000000, 0);
    this.meshObj = new THREE.Mesh(this.geo, this.basic);
    this.meshObj.frustumCulled = false;
    this.scene.add(this.meshObj, this.sun, this.ambient, this.outline, this.overlayGroup);
    this.outline.visible = false; this.outline.renderOrder = 10; this.outline.frustumCulled = false;
    this.cam.up.set(0, 0, 1);
    // selection outline colour comes from the theme token (UI_ARCHITECTURE section 3: no colours in components)
    const css = getComputedStyle(host);
    (this.outline.material as THREE.LineBasicMaterial).color.set(css.getPropertyValue('--selection').trim() || css.color || 'white');
    this.resize();
  }

  protected onGeometry(): void {
    const g = this.geom!;
    this.geo.dispose();
    this.geo = new THREE.BufferGeometry();
    this.geo.setAttribute('position', new THREE.BufferAttribute(this.displaced, 3));
    this.geo.setAttribute('normal', new THREE.BufferAttribute(new Float32Array(3 * g.nCorners), 3));
    this.colorAttr = new Uint8Array(3 * g.nCorners).fill(40);
    this.geo.setAttribute('color', new THREE.BufferAttribute(this.colorAttr, 3, true));
    this.geo.setIndex(new THREE.BufferAttribute(g.indices, 1));
    this.meshObj.geometry = this.geo;
  }

  protected onPositions(displaced: Float32Array): void {
    const pos = this.geo.getAttribute('position') as THREE.BufferAttribute;
    if (pos.array !== displaced) (pos.array as Float32Array).set(displaced);
    pos.needsUpdate = true;
    // per-cell shading normals only matter with sun lighting; computing them for 3M corners is skipped in flat mode
    if (this.opts.lighting === 'sun') this.geo.computeVertexNormals();
    else this.normalsStale = true;
  }
  private normalsStale = false;

  protected onColors(): void {
    if (!this.cellRGB || !this.geom || !this.colorAttr) return;
    cornerColors(this.geom, this.cellRGB, this.colorAttr);
    (this.geo.getAttribute('color') as THREE.BufferAttribute).needsUpdate = true;
  }

  protected onView(): void {
    const sunLit = this.opts.lighting === 'sun';
    this.meshObj.material = sunLit ? this.lambert : this.basic;
    if (sunLit && this.normalsStale) { this.geo.computeVertexNormals(); this.normalsStale = false; }
    const [x, y, z] = this.opts.sunDir;
    this.sun.position.set(x, y, z).multiplyScalar(10);
    this.sun.intensity = Math.PI; // r155+ lights are physical units: Lambert shows colour * intensity / PI, so PI gives the full map colour facing the sun
    // night side: nearly black with the day/night toggle on, softly shaded otherwise
    this.ambient.intensity = Math.PI * (this.opts.dayNight ? 0.06 : 0.55);
  }

  protected onSelection(corners: Float32Array | null): void {
    if (!corners) { this.outline.visible = false; return; }
    this.outline.geometry.dispose();
    const g = new THREE.BufferGeometry();
    g.setAttribute('position', new THREE.BufferAttribute(corners, 3));
    this.outline.geometry = g;
    this.outline.visible = true;
  }

  protected onOverlays(list: Overlay3D[]): void {
    for (const o of [...this.overlayGroup.children]) {
      this.overlayGroup.remove(o);
      (o as THREE.Mesh).geometry?.dispose();
      ((o as THREE.Mesh).material as THREE.Material).dispose();
    }
    const blending = { normal: THREE.NormalBlending, multiply: THREE.MultiplyBlending, screen: THREE.AdditiveBlending } as const;
    list.forEach((ov, i) => {
      const color = new THREE.Color(ov.style.color);
      const common = { transparent: true, opacity: ov.style.opacity, blending: blending[ov.style.blend] };
      if (ov.kind === 'segments') {
        if (ov.positions.length === 0) return;
        const g = new THREE.BufferGeometry();
        g.setAttribute('position', new THREE.BufferAttribute(ov.positions, 3));
        // emphasis (river discharge, arrow strength) as brightness: WebGL lines are one pixel wide
        const col = new Float32Array(ov.positions.length);
        for (let v = 0; v < ov.weight.length; v++) {
          const k = 0.55 + 0.45 * ov.weight[v];
          col[3 * v] = color.r * k; col[3 * v + 1] = color.g * k; col[3 * v + 2] = color.b * k;
        }
        g.setAttribute('color', new THREE.BufferAttribute(col, 3));
        const line = new THREE.LineSegments(g, new THREE.LineBasicMaterial({ vertexColors: true, depthWrite: false, ...common }));
        line.renderOrder = 1 + i; line.frustumCulled = false;
        this.overlayGroup.add(line);
      } else {
        if (ov.indices.length === 0) return;
        const g = new THREE.BufferGeometry();
        g.setAttribute('position', this.geo.getAttribute('position')); // shares the relief-displaced corners of the cell polygons
        g.setIndex(new THREE.BufferAttribute(ov.indices, 1));
        const m = new THREE.Mesh(g, new THREE.MeshBasicMaterial({ color, depthWrite: false, polygonOffset: true, polygonOffsetFactor: -2, polygonOffsetUnits: -2, ...common }));
        m.renderOrder = 1 + i; m.frustumCulled = false;
        this.overlayGroup.add(m);
      }
    });
  }

  protected onResize(w: number, h: number, dpr: number): void {
    this.renderer.setPixelRatio(dpr);
    this.renderer.setSize(w, h, false);
    this.cam.aspect = w / h;
    this.cam.updateProjectionMatrix();
  }

  protected draw(): void {
    const R = this.geom!.radius;
    const f = cameraFrame(this.camera, R);
    const { near, far } = clipPlanes(this.camera, R);
    this.cam.near = near; this.cam.far = far; this.cam.updateProjectionMatrix();
    this.cam.position.set(f.pos[0], f.pos[1], f.pos[2]);
    this.cam.lookAt(this.camera.target[0] * R, this.camera.target[1] * R, this.camera.target[2] * R);
    this.renderer.render(this.scene, this.cam);
  }

  protected onDispose(): void {
    this.geo.dispose(); this.basic.dispose(); this.lambert.dispose();
    this.outline.geometry.dispose();
    this.onOverlays([]);
    this.renderer.dispose();
  }
}
