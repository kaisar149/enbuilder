"""Python port of check_hydro() in tests/part4b2.rs (lake / rim / river invariants) run on the mirror worlds.   python mirror_4b2_invariants.py"""
import numpy as np
from mirror_4a2e import sphere_world, plane_world, cube_world
from mirror_4b1 import terrain, ocean_cells, flood, bl_of
from mirror_4b2 import hydrology, fields_for

def invariants(name, W, z, ocean, P, E, void=False, mind=0.5, **kw):
    n = W['n']; bl = bl_of(W); H = hydrology(W, z, ocean, P, E, void=void, mind=mind, **kw)
    outlet = [bool(ocean[i] or (void and bl[i] > 0)) for i in range(n)]
    F = flood(W, z, outlet); filled = F['filled']; dep = [filled[i] - F['z'][i] for i in range(n)]
    uid, units, depth, fd, qout = H['uid'], H['units'], H['depth'], H['fd'], H['qout']
    for c in range(n):
        assert (uid[c] >= 0) == (dep[c] > 0)
        assert 0 <= depth[c] <= dep[c] + 1e-9 and z[c] + depth[c] <= filled[c] + 1e-9
    nspill = 0
    for u, cells in enumerate(units):
        level = filled[cells[0]]
        for c in cells:
            for j in W['adj'][c]:
                if uid[j] != uid[c]: assert filled[j] >= level, (name, 'neighbour below spill height')
        exits = [c for c in cells if fd[c] >= 0 and uid[fd[c]] != u]; assert exits
        for c in exits:
            r = fd[c]; assert filled[r] == level and dep[r] == 0, (name, 'exit not at rim')
        if qout[u] > 0:
            nspill += 1
            for c in cells: assert abs(depth[c] - (0 if dep[c] < mind else dep[c])) < 1e-9, (name, 'spilling lake not full')
        else:
            wet = [c for c in cells if depth[c] > 0]
            if wet:
                h = z[wet[0]] + depth[wet[0]]
                for c in wet: assert abs(z[c] + depth[c] - h) < 1e-9 * max(1, abs(h)), (name, 'tilted surface')
                assert h <= level + 1e-9
    print(f'{name:12s} ok  units={len(units)} spilling={nspill} unresolved={len(H["unresolved"])}')

for L in (3, 4):
    W = sphere_world(L); z = terrain(W, 11); P, E = fields_for(W, z, 31); invariants(f'sphere{L}', W, z, ocean_cells(W, z), P, E)
W = cube_world(14); z = terrain(W, 12); P, E = fields_for(W, z, 32); invariants('cube', W, z, ocean_cells(W, z), P, E)
W = plane_world(40, 24); z = terrain(W, 13); P, E = fields_for(W, z, 33); invariants('plane_wall', W, z, ocean_cells(W, z), P, E)
W = plane_world(40, 24, wrap=True); z = terrain(W, 13); oc = ocean_cells(W, z)
for nm, p0, kw in (('wrap', 3e-5, {}), ('wrap_wet', 3e-4, {}), ('wrap_dry', 3e-7, {}), ('wrap_lf0', 3e-5, dict(lf=0.0)), ('wrap_c0', 3e-5, dict(coeff=0.0)), ('wrap_c1', 3e-5, dict(coeff=1.0))):
    P, E = fields_for(W, z, 33, p0=p0); invariants(nm, W, z, oc, P, E, **kw)
W = plane_world(40, 24); z = terrain(W, 14); P, E = fields_for(W, z, 34); invariants('plane_void', W, z, [False] * W['n'], P, E, void=True)
z = np.array([1000.0 + ((i * 37) % 101) for i in range(W['n'])]); P, E = fields_for(W, z, 35); invariants('closed', W, z, [False] * W['n'], P, E)
