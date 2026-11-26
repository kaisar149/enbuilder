import { useEffect, useRef } from 'react';
import { isJobRunning, layerDecl, variationThumbField, type FieldData, type MeshData, type Schema } from 'wb-client';
import { useStore } from '../state/hooks';
import { renderThumbnail } from '../viewport/thumbnail';
import type { PanelProps } from './components';

const THUMB_W = 120;

function Thumb({ mesh, schema, field, label, onOpen }: { mesh: MeshData; schema: Schema; field: FieldData | undefined; label: string; onOpen(): void }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const cv = ref.current;
    if (!cv || !field) return;
    const img = renderThumbnail(mesh, field, layerDecl(schema, field), { biomes: schema.biomes }, THUMB_W);
    cv.width = img.width;
    cv.height = img.height;
    const g = cv.getContext('2d');
    if (g) g.putImageData(new ImageData(img.rgba as unknown as Uint8ClampedArray<ArrayBuffer>, img.width, img.height), 0, 0);
  }, [mesh, schema, field]);
  return (
    <button type="button" className="thumb" aria-label={label} title={label} onClick={onOpen}>
      <canvas ref={ref} />
      <span>{label}</span>
    </button>
  );
}

export function VariationsPanel({ app, ctl }: PanelProps) {
  const schema = useStore(app.store, (s) => s.schema);
  const world = useStore(app.store, (s) => s.world);
  const mesh = useStore(app.store, (s) => s.mesh);
  const variations = useStore(app.store, (s) => s.variations);
  const fields = useStore(app.store, (s) => s.variationFields);
  const job = useStore(app.store, (s) => s.jobs.current);
  const running = useStore(app.store, isJobRunning);
  const ui = useStore(ctl.params.ui, (s) => s);
  const seedDef = schema?.params.find((p) => p.name === 'seed');
  const thumbField = variationThumbField(schema, variations);
  useEffect(() => {
    if (!thumbField) return;
    for (const it of variations?.items ?? []) void app.ensureVariation(it.index, thumbField).catch(() => undefined);
  }, [app, variations, thumbField]);
  if (!schema || !world || !mesh) return <p className="note">Waiting for the world.</p>;
  const seed = ctl.params.value('seed') ?? 0;
  const busyVar = running && job?.kind === 'variations';
  return (
    <div className="stack">
      <div className="row">
        <label className="field grow">
          <span>Seed</span>
          <input
            type="number" step={1} min={seedDef?.min} max={seedDef?.max} value={Number.isFinite(seed) ? seed : ''} aria-label="Seed"
            aria-invalid={ui.errors.seed ? true : undefined}
            onChange={(e) => ctl.params.edit('seed', e.target.value === '' ? Number.NaN : Number(e.target.value))} onBlur={() => void ctl.params.commit('seed')}
            onKeyDown={(e) => { if (e.key === 'Enter') void ctl.params.commit('seed'); }}
          />
        </label>
        <button type="button" className="btn align-end" onClick={() => void ctl.params.randomSeed()}>Random seed</button>
      </div>
      {ui.errors.seed && <p className="note error" role="alert">{ui.errors.seed}</p>}
      <button type="button" className="btn" disabled={running} onClick={() => void app.command({ type: 'variations', count: 6, coarse: true }).catch(() => undefined)}>Generate 6 variations</button>
      {busyVar && <p className="note" role="status">{job?.stage || 'starting'} ({Math.round((job?.fraction ?? 0) * 100)}%)</p>}
      {variations && variations.items.length > 0 && (
        <>
          <p className="note">Quick low-detail solves of the same world with other seeds. Click one to use its seed and solve it fully.</p>
          <div className="thumbs" role="list" aria-label="Variations">
            {variations.items.map((it) => (
              <div role="listitem" key={it.index}>
                <Thumb mesh={mesh} schema={schema} field={thumbField ? fields[`${it.index}:${thumbField}`] : undefined} label={`Seed ${it.seed}`} onOpen={() => void ctl.params.openVariation(it.index).catch(() => undefined)} />
              </div>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
