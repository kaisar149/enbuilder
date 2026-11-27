# HANDOFF: Part 9e (overlays, layer stack, inspector). Checkpoint 4: code, tests and docs complete

## How to resume
Extract `part9e_changes_cp4.tar.gz` over the project root. Sandbox toolchain (no rustup access): conda-forge `rust` + `rust-std-x86_64-unknown-linux-gnu` 1.100.0 `.conda` files extracted with python `zstandard` into /tmp/rustenv; `export PATH=/tmp/rustenv/bin:$PATH`. Then `npm install`, `cargo build --release -p wb-server`, `npm run test:client`, `npm run test:ui`, `cargo test --release -p wb-app`.

## Done (all 9e deliverables)
Core queries/schema/presets, wb-client cache, pure UI modules, 2D/3D renderers, panels, controller, tests, UI_NOTES.md (9e section), ARCHITECTURE_9.md section 14, USER_GUIDE_9E.md.

## Test status (this session)
wb-client 34/34, ui-web 79/79 (two clean reruns), wb-app 13+6+5+6, typecheck clean. One ui-web run failed once early (same code, did not reproduce on 3 later runs); cause not identified.

## Not done / open
- Overlay drawing never looked at in a real browser or on a GPU (only pure geometry + stub renderer tested). Do a visual pass: Solve, Hydrology, Climate, streaks, 3D view.
- Inspector shows values only; "why is it dry" explanation needs the later explainer part.
- Part 9f (paint/lock/diff) not started.
