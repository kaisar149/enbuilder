# HANDOFF: Part 9d (3D viewer). Checkpoint 3

## How to resume
Extract `part9d_changes.tar.gz` over the project root (only files that differ from your zip; `CHANGED_FILES_9D.txt` lists them), then `npm install`
(adds `three@0.160.0`, `@types/three`). If interrupted, tell the next session: "continue Part 9d from HANDOFF_PART9D.md".

## Sandbox
Rust toolchain was installed this session: `export PATH=/opt/rustenv/bin:$PATH` (micromamba env). `cargo build -p wb-server` is slow the first time.
No GPU/browser here: WebGL cannot be run, so frame rate on a real GPU is NOT measured here (see "Frame rate" in the final notes).

## Done (ui-web only; no Rust, no wb-client change so far)
- `src/viewport/types.ts`: `OrbitCamera`, `AnyCamera`, `isOrbit`, `View3DOptions`, `DEFAULT_VIEW3D`, `Viewport3D` (extends Viewport: setElevation, setView, setSelection, focusCell, resetView, frameStats). `Viewport.setCamera/getCamera/onCameraChange` now take `AnyCamera`; `MapViewport2D` ignores orbit cameras.
- `src/viewport/geometry3d.ts` (pure): mesh -> flat-coloured cell polygons (circumcentres of cell+2 consecutive neighbours; works for hex/pent and square grids; cube folds unfolded; plane edge mirrored). Relief by corner-incident cells (no cracks). `buildCellGeometry`, `cellRelief`, `displaceCorners`, `cornerColors`, `cellTop`, `surfaceNormal`, `heightAbove`, `toSurface`.
- `src/viewport/orbit.ts` (pure): per-shape limits, home camera, orbit/zoom/pan, `cameraFrame`, `rayFromPixel`, `projectPoint`, `focusOn`, `lerpCamera`, `clipPlanes`.
- `src/viewport/pick3d.ts` (pure): `pickRay` analytic base-shape hit + relief shell march, graph walk to the cell.
- `src/viewport/sun.ts` (pure): `sunDirection(params, plane, season, timeOfDay)` same geometry as the core insolation.
- Tests: `test/synth.ts` (sphere/cube/plane mesh generators of any size), `test/part9d_geometry.test.ts` (5), `test/part9d_pick.test.ts` (8). Run `cd ui-web && npx vitest run test/part9d_*.test.ts`.

## Checkpoint 2 (this session): renderer layer done and tested
- `src/viewport/BaseViewport3D.ts`: everything except drawing (canvas, pointer/wheel/key input, orbit camera + focus animation, `pick` via pickRay, hover/pick events, colour per cell via the 2D `cellColors`, relief, selection, frame stats). Renderers implement 8 `on*`/`draw` hooks only.
- `src/viewport/MeshViewport3D.ts`: Three.js renderer, the ONLY file importing `three`. Colour management off + linear output so cell colours equal 2D bytes. Sun lighting = Lambert + DirectionalLight (night ambient 0.06 with dayNight on, 0.55 off). Selection = white LineLoop.
- `src/viewport/StubViewport.ts`: draws nothing, logs hook calls (swap proof, also WebGL fallback).
- `src/viewport/factory.ts`: `createViewport3D(host, 'three'|'stub')`, lazy `import()` of three, falls back to the stub without WebGL.
- `test/part9d_viewport.test.ts` (7, jsdom): stub swap, pick on sphere/cube/plane through the viewport, camera/focus/selection, 2D/3D colour parity (sphere corners, plane raster pixels).
- Total 9d tests now 20 passing: `cd ui-web && npx vitest run test/part9d_*.test.ts`. `npx tsc --noEmit -p .` clean.
- Install: `npm install` at project root (three@0.160.0 is in ui-web/package.json).

## Checkpoint 3: UI wired, docs written
- New: `viewport/view3d.ts` + `view3d.config.json` (relief field as data), `viewport/bench.ts`, `panels/View3DPanel.tsx`, `test/part9d_viewport.test.ts` (7), `test/part9d_ui.test.tsx` (3, real wb-server), `test/gl/harness.ts` + `test/gl/run.mjs` (real Three.js in headless Chromium, manual).
- Changed: `state/controller.ts` (view, view3d, hover tooltip via get_cell, benchmark), `MapView.tsx` (2D/3D switch, tooltip, FPS), `panels/registry.ts`, `panels/components.tsx`, `app.css`, `theme.css` (--selection), `UI_NOTES.md`, `ARCHITECTURE_9.md` (section 13), new `USER_GUIDE_9D.md`.
- Results: 53/57 ui tests pass in sandbox; the 4 failing are old 2D wb-snap parity tests (need `cargo build --release -p wb-app`). Real WebGL (software) colour parity 360/360.
- Sandbox: Rust via micromamba into /opt/rustenv (`export PATH=/opt/rustenv/bin:$PATH`); Chromium at /opt/pw-browsers; run `node test/gl/run.mjs /tmp/gl_out`.

## Still to do
1. Run the full suite once with the release wb-snap built; confirm the 4 old tests pass.
2. MEASURE the GPU frame rate on your machine (Measure frame rate button, or test/gl/run.mjs without swiftshader flags) and fill the blank in UI_NOTES.md. Not measured here (software GL gave 1.2-2.5 fps).
3. static.test: add a rule that only MeshViewport3D.ts imports `three`.
4. Optional: schema flag for the height field instead of view3d.config.json.

## Decisions
- Cells are polygons with flat colour (identical to 2D). Cube relief direction is radial so folds stay joined.
- Relief height = max(elevation,0) x exaggeration x unitsPerMetre (sea stays flat). `unitsPerMetre` = mesh radius / (radius_km*1000) computed by the UI.
- Picking never triangle-tests: base shape + relief march, so cost is independent of mesh size.
