# HANDOFF: Part 9a COMPLETE (checkpoint 3)
## Decisions
- Part 8 skipped (user decision). New notes: `ARCHITECTURE_9.md`, `UI_NOTES.md` (ARCHITECTURE.md is NOT edited).
- Workspace: root Cargo.toml has `[workspace] members = [".", "wb-app"]`. The zip ships the REAL root file (edition 2024 plus workspace lines). Sandbox used edition 2021 and cargo 1.75; if building on a similar old toolchain, set edition = "2021" temporarily.
## Done (all Part 9a acceptance tests pass: 13 tests in wb-app/tests/part9a.rs)
- wb-app: api, params, world, schema, session (jobs, cancel, undo/redo), png, raster (colormaps, sphere/cube/plane projections, legend), bin `wb-snap`.
- Simulation crate change: only `Scheduler::module_params` (read-only) in src/scheduler.rs.
## Verify on your machine
`cargo test -p wb-app` (first run on a new machine: if only `snapshot_hashes.json` differs, regenerate with `UPDATE_GOLDEN=1`), then the demo script in UI_NOTES.md.
## Next: Part 9b (proposed)
1. Adapter: local WebSocket server (JSON text for commands/queries/events, binary frames for mesh and fields) in a new crate `wb-server`.
2. `wb-client`: framework-free TypeScript (API client, store, selectors), tested under Node against the server.
3. Extra commands as needed: `get_field` for wind (vector layers), `save_world`/`load_world`.
4. Update ARCHITECTURE_9.md and UI_NOTES.md; list any simulation-crate change.
