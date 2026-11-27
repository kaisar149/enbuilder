# User guide: painting, locks and realism (Part 9f)

Solve a world first so the climate can react to your edits.

- **Paint on the map:** open *Paint tools* and press *Paint on the map*. Drag on the map (2D or 3D) with a brush. Shift-drag or right-drag still moves the view; Escape cancels a stroke in progress.
- **Brushes:** *Raise* and *Lower* change terrain (dig below sea level to make a lake or sea), *Smooth* and *Flatten* tidy it, *Warmer* and *Cooler* add a temperature bias, *Wetter* and *Drier* a rainfall bias, *Biome* overrides the biome, *Eraser* returns a layer to the physics. Set radius (shown as an outline of the cells that will change), strength, falloff and amount.
- **Preview, then refined:** when you release an elevation stroke the map updates in a fraction of a second (badge: "Refining: quick preview"), then a more careful pass ("Refining: refined pass") ends on "Refined". Use *Recalculate* for a *Full solve* when you want the complete result. A new stroke cancels the pass in progress.
- **Realism:** each field has a slider. 0 shows what you painted, 100 shows only the physics, values in between blend them. Moving it is instant. *Physics only* and *Painted* in the same panel show each side by itself.
- **Locks:** choose *Lock*, pick the field, and paint over the area. Locked cells are hatched, and the solver treats them as fixed. Painting over them changes nothing, and the panel tells you how many cells were skipped. *Unlock* releases them.
- **History:** strokes, locks, realism changes and parameter changes are one list. Undo removes the last one, and undoing a stroke restores the world exactly.

Try it: raise a ridge across the prevailing wind, release, and look at Rainfall for the dry side behind it. Lock a valley, paint around it, and check that the valley did not change.

Limits: the result is a reduced-order model, so the preview is approximate (about 0.3 K and 5 % rainfall from the refined result on the test world). The 3D view shows locks as a plain tint, not stripes. Drawing was checked in a test environment, not yet on a real GPU.
