// Static architecture checks: no hard-coded field names, no colours outside the theme file. They need no server.
import { describe, expect, it } from 'vitest';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const src = resolve(here, '../src');
const walk = (d: string): string[] => readdirSync(d).flatMap((f) => (statSync(join(d, f)).isDirectory() ? walk(join(d, f)) : [join(d, f)]));
const files = walk(src);
const schema = JSON.parse(readFileSync(resolve(here, '../../wb-app/tests/golden/schema.json'), 'utf8'));

describe('data-driven UI', () => {
  it('ui-web source names no schema field (colormaps.ts is exempt: colormap names are a separate vocabulary)', () => {
    const names: string[] = schema.fields.map((f: any) => f.name);
    const hits: string[] = [];
    for (const f of files.filter((x) => /\.(ts|tsx)$/.test(x) && !x.endsWith('colormaps.ts'))) {
      const text = readFileSync(f, 'utf8');
      for (const n of names) if (new RegExp(`['"\`]${n}['"\`]`).test(text)) hits.push(`${f.replace(src, '')}: '${n}'`);
    }
    expect(hits).toEqual([]);
  });

  it('colours, fonts and sizes live in theme.css only', () => {
    const offenders: string[] = [];
    for (const f of files.filter((x) => /\.(tsx|css)$/.test(x) && !x.endsWith('theme.css'))) {
      const text = readFileSync(f, 'utf8');
      if (/#[0-9a-fA-F]{3,8}\b/.test(text)) offenders.push(f.replace(src, '') + ' (hex colour)');
      if (f.endsWith('.css') && /\brgba?\(/.test(text)) offenders.push(f.replace(src, '') + ' (rgb())');
    }
    expect(offenders).toEqual([]);
  });

  it('only MeshViewport3D.ts imports three', () => {
    const hits = files.filter((x) => /\.(ts|tsx)$/.test(x) && /from ['"]three['"]/.test(readFileSync(x, 'utf8'))).map((x) => x.replace(src, ''));
    expect(hits).toEqual(['/viewport/MeshViewport3D.ts']);
  });

  it('every registry component exists, and every panel can be moved by data alone', async () => {
    const { PANEL_REGISTRY, resolveLayout } = await import('../src/panels/registry');
    const { PANEL_COMPONENTS } = await import('../src/panels/components');
    for (const p of PANEL_REGISTRY) expect(PANEL_COMPONENTS[p.component], p.id).toBeTruthy();
    const moved = resolveLayout(PANEL_REGISTRY, { legend: 'right', solve: 'hidden' });
    expect(moved.right.map((p) => p.id)).toContain('legend');
    expect([...moved.left, ...moved.right, ...moved.bottom].some((p) => p.id === 'solve')).toBe(false);
  });
});
