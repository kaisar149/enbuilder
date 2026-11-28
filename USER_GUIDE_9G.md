# User guide: seeing what an edit changed (Part 9g)

You need two versions of the world: solve it, make an edit (paint terrain, add a lock) and let it refine. Every solve and refresh is kept (the last 8).

- **Diff:** press *Diff* in the map toolbar (or *Show diff* in the Diff panel). The map shows the change between the two chosen versions: blue = lower, red = higher, white = no change. *Before* and *After* choose the versions (default: the one before the latest, and the latest). *Latest two versions* goes back to that default.
- **Field:** pick which field to look at; the list shows how many cells changed in each. Biomes show changed / unchanged only.
- **Ripple:** *Play ripple* reveals the changed cells from the edit outward. Drag *Time* to scrub. It works in 2D and 3D, and you can switch views while it plays.
- **Colour range:** the slider sets how large a change gets full colour; lower it to see small changes.
- **Before / After / Swipe / Flip:** look at the real values before or after instead of the difference. *Swipe* shows after on one side of a handle and before on the other (it sweeps by longitude on a globe). *Flip* toggles between them.
- **Hide changes under 1%:** on by default. Re-solving a world never gives bit-identical numbers, so tiny differences are hidden; the summary says how many. Switch it off to see everything.
- **Why did this change?** With Diff on, click a cell. The panel lists the chain of causes, the effect first and the origin last, with the module behind each step. Click a step to light up the cells it came from.
- **Summary:** cells changed, largest change, how far it reached, and a table per field.

Honest limits: the chain is what the model measured, and picking among several possible causes is a ranking, not a proof. A change often reaches far (pressure and moisture couple the whole planet in this model), so the chain for a far-away cell can be short, "global coupling", instead of a neat story. A quick *preview* result carries its own approximation error, so compare refined with refined. The ripple order is distance from the edit, not simulated time.
