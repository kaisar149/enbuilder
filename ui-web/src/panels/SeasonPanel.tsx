import { metaOf, prettyName, seasonLabel, seasonVariant, seasonsAvailable } from 'wb-client';
import { useStore } from '../state/hooks';
import type { PanelProps } from './components';

export function SeasonPanel({ app, ctl }: PanelProps) {
  const schema = useStore(app.store, (s) => s.schema);
  const world = useStore(app.store, (s) => s.world);
  const layer = useStore(ctl.ui, (s) => s.layer);
  const season = useStore(ctl.ui, (s) => s.season);
  if (!schema?.seasons || !world) return null;
  const count = schema.seasons.count;
  const name = layer ? prettyName(layer) : 'this layer';
  if (!world.solved) return <p className="note">Solve first to get seasons.</p>;
  if ((world.params.year_length_days ?? 0) <= 0) return <p className="note">This world has no orbit, so there are no seasons.</p>;
  if (!layer || !seasonVariant(schema, layer)) {
    return <p className="note">{layer && metaOf(schema, layer) ? `${name} has no seasonal version.` : 'Pick a layer.'} Seasons are stored for: {Object.keys(schema.seasons.variants).map(prettyName).join(', ')}.</p>;
  }
  if (!seasonsAvailable(schema, world, layer)) return <p className="note">Season data is not available yet. Solve again.</p>;
  return (
    <div className="stack">
      <label className="field">
        <span>{season === null ? 'Yearly average' : seasonLabel(season, count, world.params.year_length_days)}</span>
        <input type="range" aria-label="Season" min={0} max={count - 1} step={1} value={season ?? 0} onChange={(e) => ctl.setSeason(Number(e.target.value))} />
      </label>
      <div className="row">
        <button type="button" className="btn" disabled={season === null} onClick={() => ctl.setSeason(null)}>Yearly average</button>
        <button type="button" className="btn" disabled={season === null} onClick={() => ctl.setSeason(((season ?? 0) + 1) % count)}>Next season</button>
      </div>
      <p className="note">Showing stored seasons; no new solve is needed.</p>
    </div>
  );
}
