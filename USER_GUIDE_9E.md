# User guide: overlays, layer stack and the cell inspector (Part 9e)

Solve a world first (rivers, wind and currents need the solved fields).

- **Presets (Layer stack panel):** one click sets the map layer and the overlays. *Terrain*: relief, coast, glaciers. *Climate*: rainfall with wind arrows. *Hydrology*: relief with rivers, lakes, coast. *Ocean*: sea temperature with currents.
- **Overlays:** wind and current arrows (longer = faster, fewer when zoomed out), rivers (thicker = more water), lakes (filled), coastline and glacier outlines. Each has a visibility toggle, opacity, blend mode, and **Solo** to look at one layer alone.
- **Reorder:** move an overlay up or down; this only changes which is drawn on top. The map layer is always at the bottom.
- **Animated streaks:** switch on for moving wind and current lines.
- **Cell inspector:** click a cell to see every value grouped by module, with units and the 12 seasonal values where they exist. **Pin** a cell, then click another to compare side by side with differences. Values are exactly what the model computed; nothing is recalculated in the panel.

Try it: pick *Climate*, click a dry cell on the downwind side of a mountain range, then pin it and click a wet windward cell to see why they differ.

Limits: the inspector shows values, not a causal chain yet (the explainer is a later part). Overlay drawing was not checked on a real GPU in this session.
