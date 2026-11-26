# User guide: changing the laws of your world (Part 9c)

## The panels
**Preset** (left): pick Earth like, Desert, Waterworld, Tidally locked or Ice age. A preset sets every law, so picking one discards your earlier tweaks (you can Undo it). Under the picker a line says *Matches Earth like* (green) or *Changed from Earth like: 2 settings* (red) with the list of what you changed. *Back to ...* restores the preset. Shape, resolution, radius and seed are not counted as changes, because they say which world it is, not how its physics behave.

**World laws** (left): one slider per law (gravity, day length, year length, axial tilt, star brightness, ocean fraction ...), each with its unit, its default and a Reset button. Drag as much as you like; one change is sent when you stop. You can also type a number and press Enter. Nothing is recalculated until you press **Solve** (the map keeps showing the last solution, and the Solve panel says when it is out of date). The *Solver* group is collapsed by default.

If you type a value outside the allowed range, the app does not quietly change it: the message from the core (for example `parameter 'axial_tilt_deg' must be in [0, 90], got 120`) appears under the control and your number stays so you can fix it.

**Seed and variations** (left): the seed decides the random terrain. *Random seed* picks one. *Generate 6 variations* solves six other seeds quickly at low detail and shows six thumbnails of the biome map. Click a thumbnail to use its seed and solve that world fully. The same seed always gives the same world.

**Season** (right): after a solve, for layers that vary through the year (temperature, precipitation, soil moisture) drag the slider to view any of the 12 stored seasons. This does not recalculate anything. *Yearly average* returns to the normal map. Worlds without an orbit (year length 0, such as tidally locked) have no seasons.

**History** (right): every change in order, with Undo and Redo. Undone steps are struck through until you make a new change.

## Try this
1. Start with Earth like and Solve.
2. Set axial tilt to 60 and Solve again. Open the Season slider on Temperature: summers and winters now differ much more at high latitudes.
3. Pick Waterworld, Solve, then Generate 6 variations and click one.
4. Use Undo to walk back to the 60 degree Earth.

## Limits
Variations are quick drafts (one year of spin-up), so details differ slightly from the full solve of the same seed. Greenhouse, evaporation and erosion controls will appear once the simulation exposes them.
