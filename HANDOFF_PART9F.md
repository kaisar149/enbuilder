# HANDOFF: Part 9f  (STATUS: all planned deliverables done at cp4; wb-app 11 part9f tests, wb-client 47, ui-web 102, all green) (paint tools, locks, realism slider, preview modes)  -- LIVE FILE, updated at every checkpoint

## How to resume
Extract the latest `part9f_cpN.tar.gz` over the project root (it contains the whole source tree minus target/ and node_modules/).
Toolchain (no rustup): conda-forge `rust-1.100.0` (linux-64) + `rust-std-x86_64-unknown-linux-gnu-1.100.0` (noarch) `.conda` files, extract
`pkg-*.tar.zst` inside with python `zstandard` into /tmp/rustenv; `export PATH=/tmp/rustenv/bin:$PATH`. Then `npm install`,
`cargo build --release -p wb-app -p wb-server`, `cargo test --release -p wb-app`, `npm run test:client`, `npm run test:ui`.

## Measured baseline (this sandbox, release, earthlike res 3 = 642 cells)
full solve spinup 1 = 0.65 s, spinup 2 = 1.1 s  => ~37 ms per season; res 2 (162 cells) full year 0.08 s; res 4 (2562 cells) 5.3 s.

## Design decisions (9f)
1. Paint layers live in `wb-app` (`paint.rs`, pure): per field `value[]` + `mask[]`; fields: `elevation` (absolute m), `temperature_bias` (K delta),
   `rainfall_bias` (fractional delta of precipitation), `biome_override` (biome id). Stored SEPARATELY from computed fields.
2. Blend rule (per field realism r in [0,1] = slider/100): `effective = computed + (1-r) * mask * (painted - computed)`
   (bias layers: painted - computed = bias). r=0 -> painted as given, r=1 -> ignored, linear between. Category (biome): painted wins iff (1-r)*mask >= 0.5.
3. Strokes always edit the r=0 "painted view" (what you painted), independent of the slider. One `paint_stroke` command per stroke (batched by the client).
4. Locks: lockable output fields `elevation`, `temperature`, `precipitation`, `biome`. elevation + temperature locks are pinned inside the solver
   (FieldStore locks = boundary conditions); precipitation + biome locks hold the published value. Painting a locked cell is skipped (reported).
5. History is unified: params groups, strokes, locks and realism changes share one undo/redo stack with `kind`. Strokes keep an `Arc` snapshot of the
   solved state on the other side of the step (swap on undo/redo) so undo restores climate exactly. "Field versions equal" is implemented as
   content-hash equality, because versions must stay monotonic for client caches (documented deviation).
6. Solved state stays UNBIASED/computed; a derived "view" (bias, biome override, locks) is rebuilt on every mutation. Realism changes are instant, no re-solve.
7. Preview modes (command `refresh {mode}`, background job): `preview` = warm start from the last solve, 2 representative seasons (1 if too slow), rest of
   the year extrapolated by the mean delta; `refined` = warm start, 12 seasons; `full` = existing `solve` (now honours paint + locks).
8. Client: `tools.ts` (Tool interface + registry, brush geometry via mesh adjacency, falloff, stroke batching) and `EditController` (preview then refined).

## Implementation notes (as built, Rust core)
- Toolchain: the sandbox already had cargo 1.97 at /root/.cargo/bin; no conda install was needed. `npm install` is still required (node_modules is not in the zip).
- `wb-app/src/paint.rs` (pure): Layer{value,mask}, Edits{paint,locks,realism}, blend fns, `apply_stroke` (ops add/set/smooth/flatten/erase), `hash_f64`.
  Strokes touch -> mask=1 and store the new r=0 view value (continuous layers); biome: painted id wins where w >= old mask; erase: mask *= (1 - w*strength).
- Default realism = 0 (paint visible at once). Edits are stored per mesh_id (undoing a mesh change brings that mesh's paint back).
- `world.rs`: `EditInput{elevation, pins}`, `solve_with`, `refresh(prev, edit, Preview|Refined)`. Preview = seasons [3, 9] (second skipped if the first took > 60 ms),
  other seasons extrapolated by the mean delta; biomes reclassified; no ocean-current diagnostic. Refined = 12 seasons warm start.
- `session.rs`: View (computed -> paint -> locks) rebuilt after every mutation, versions bump only when the content hash changes (monotonic).
  Staleness = params flag OR elevation-view hash != the hash the solved state used. History is `Step::{Params,Stroke,Lock,Realism}`; elevation strokes swap an
  Arc'd SolvedState on undo/redo. Strokes/locks/realism cancel any running job; a refresh supersedes only a running refresh (solve/variations -> job_running).
- API additions (additive, API_VERSION stays 1): commands `paint_stroke`, `set_lock`, `set_realism`, `refresh`; queries `get_field_view` (blended|computed|painted),
  `get_paint`, `get_locks`; Ack.detail; world_info.edit {generation, realism, locks, paint, quality, last_refresh}, field.hash, job.mode, history entries have `kind`;
  get_cell.edit_state; schema.edit (tools, brush, realism, refresh_modes as data). Goldens regenerated (world_info, schema).
- Measured (release, res 3): preview 64 ms (2 seasons), refined 356 ms (12), preview vs refined temperature MAE 0.28 K (max 1.7), precipitation MAE 5.4 % of mean;
  refined vs full MAE 0.004 K. Tests: `cargo test --release -p wb-app --test part9f` (11 tests).

## Status
- [x] Rust core: paint.rs, api, session, world, schema, tests (all wb-app + wb-server tests green at cp1)
- [x] wb-client (cp2): types (9f commands/queries/EditInfo/EditSchema/PaintData/LocksData), client methods (paintStroke, setLock, setRealism, refresh, getFieldView, getPaint, getLocks),
      frames decoders, WbApp (refresh job tracking with `mode`, ensurePaint/ensureLocks/ensureFieldView cached per edit generation, refreshAndWait),
      `tools.ts` (falloff, brushDab via adjacency BFS, cellPath for fast drags, StrokeBuffer, ToolRegistry.fromSchema), `edit.ts` (EditController: pointerDown/Move/Up,
      ONE command per stroke, preview->refined chain only for solver inputs = elevation strokes + elevation/temperature locks), test/part9f.test.ts (12 tests, 46 total green)
- [x] ui-web (cp3): viewport tool mode (`setToolMode`, `onStroke` on 2D + 3D base; shift/right-drag still navigate), `tools/overlays.ts` (brush ring = outline mask of the
      cells the brush touches, stroke preview = fill mask, lock hatch = fill mask with `pattern:'hatch'`, all ordinary overlay items; 3D draws hatch as translucent fill),
      MapController (`edit`, `editOn`, `fieldView`, `handleStroke`, `setEditMode`, `setFieldView`, lock fetching), panels ToolsPanel / RealismPanel / RefreshBadge,
      HistoryPanel shows kinds, theme tokens --overlay-brush/-stroke/-lock. Tests: part9f_tools (11), part9f_stroke3d (4), part9f_ui (8, real server demo script).
      ui-web 102 tests, wb-client 46, typecheck + vite build green. The static guard forced all output-field names to come from `schema.edit` (none hard-coded in ui-web/src).
- [x] Docs (cp4): UI_NOTES (Part 9f section incl. tool interface, paint format, blend rule, thresholds), ARCHITECTURE_9 s15, USER_GUIDE_9F.md
- [ ] Remaining (optional): real-browser/GPU check of the demo script; schema flag for "needs refresh"; hatch in 3D; update ARCHITECTURE.md (main) summary line for 9f

## Client design notes (cp2)
- Additive strokes: client scales weights by brush strength (server ignores strength for `add`); set/smooth/flatten/erase pass `strength`.
- Drag dabs are spaced at 25% of the radius; a stroke is a weight-max union of dabs (same rule as the core), sent once on pointer up.
- Locks tool locks every cell the brush touched (weight > 0). Locks hold their value regardless of realism (tests exclude them from blend comparisons).
- Bias / biome strokes and realism changes are applied as an instant derived view; only elevation paint and elevation/temperature locks start refresh chains.
- Next: ui-web (read /mnt/skills/public/frontend-design first): Tools panel, brush ring, lock hatch overlay, realism sliders, history list, refine badge.

## UI design notes (cp3)
- Painting is opt-in ("Paint on the map" toggle); off = the map navigates exactly as before.
- Realism slider commits on pointer/key release (not per tick) so dragging does not flood the undo stack.
- Field view switch (Blend / Physics only / Painted) is UI-only state; it fetches `get_field_view` for the shown output layer.
- Known limitation: wb-client `edit.ts` hard-codes which edits need a solver refresh (elevation strokes, elevation/temperature locks); ideally the schema would declare it.
- 3D renderer cannot hatch; locks are shown as translucent fill there.
- Not yet verified in a real browser (jsdom + stub renderer only). `npm run build:ui` works; serve with wb-server to try the demo script.
