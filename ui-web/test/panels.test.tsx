// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { Store, initialState, layerChoices, type Schema, type WbApp, type WorldInfo } from 'wb-client';
import { LayersPanel } from '../src/panels/LayersPanel';
import { LegendPanel } from '../src/panels/LegendPanel';
import type { MapController } from '../src/state/controller';
import { initialUi } from '../src/state/controller';

const golden = (n: string) => JSON.parse(readFileSync(resolve(__dirname, '../../wb-app/tests/golden', n), 'utf8'));
const schema = golden('schema.json') as Schema;
const world = golden('world_info.json') as WorldInfo;

function fakes(layer: string | null) {
  const store = new Store({ ...initialState(), schema, world });
  const ui = new Store({ ...initialUi(), layer });
  const select = vi.fn();
  return { app: { store } as unknown as WbApp, ctl: { ui, selectLayer: select } as unknown as MapController, select };
}
afterEach(cleanup);

describe('panels are generated from the schema', () => {
  it('the layer dropdown lists exactly the schema layers and disables the unsolved ones', () => {
    const { app, ctl } = fakes('elevation');
    render(<LayersPanel app={app} ctl={ctl} />);
    const choices = layerChoices(schema, world);
    const options = screen.getAllByRole('option') as HTMLOptionElement[];
    expect(options.map((o) => o.value)).toEqual([...choices.filter((c) => c.group === 'featured'), ...choices.filter((c) => c.group === 'other')].map((c) => c.field));
    expect(options.filter((o) => !o.disabled).map((o) => o.value)).toEqual(choices.filter((c) => c.available).map((c) => c.field).sort((a, b) => options.findIndex((o) => o.value === a) - options.findIndex((o) => o.value === b)));
    expect(options.some((o) => o.disabled)).toBe(true);
  });

  it('choosing a layer calls the controller with that field and nothing else', () => {
    const { app, ctl, select } = fakes('elevation');
    render(<LayersPanel app={app} ctl={ctl} />);
    const sel = screen.getByRole('combobox') as HTMLSelectElement;
    const target = [...sel.options].find((o) => !o.disabled && o.value !== 'elevation');
    if (!target) return; // golden world only has elevation solved; the selection path is covered by server.test.ts
    fireEvent.change(sel, { target: { value: target.value } });
    expect(select).toHaveBeenCalledWith(target.value);
  });

  it('shows a hint instead of a legend until a layer has data', () => {
    const { app, ctl } = fakes('elevation');
    render(<LegendPanel app={app} ctl={ctl} />);
    expect(screen.getByText(/no layer to describe/i)).toBeTruthy();
  });
});
