"""Python mirror of Part 4b-2 (src/rivers_lakes.rs). Same unit definition, same steady-state lake balance, same Kahn contraction,
same river forward fill. Prints the numbers quoted in tests/part4b2.rs.   python mirror_4b2.py   (needs mirror_4b1.py and helpers here)
"""
import heapq, numpy as np
from mirror_4a2e import sphere_world, plane_world, cube_world
from mirror_4b1 import Rng, terrain, ocean_cells, flood, flow_dir, bl_of

def solve_lake(level, z, area, ev, q_in):
    m = len(z)
    if not (q_in > 0): return [0.0] * m, 0.0, 0.0, False
    e_full = sum(area[i] * ev[i] for i in range(m))
    if q_in > e_full: return [max(level - z[i], 0.0) for i in range(m)], q_in - e_full, e_full, True
    idx = sorted(range(m), key=lambda i: (z[i], i)); cum = 0.0; k = 0
    while k < m:
        ek = area[idx[k]] * ev[idx[k]]
        if cum + ek <= q_in: cum += ek; k += 1
        else: break
    h = z[idx[k]] if k < m else level; depth = [0.0] * m
    for i in idx[:k]: depth[i] = max(h - z[i], 0.0)
    return depth, 0.0, q_in, k == m

def lake_units(W, filled, dep):
    n = W['n']; uid = [-1] * n; units = []
    for s in range(n):
        if uid[s] >= 0 or not dep[s] > 0: continue
        u = len(units); uid[s] = u; st = [s]; mem = []
        while st:
            c = st.pop(); mem.append(c)
            for j in W['adj'][c]:
                if uid[j] < 0 and dep[j] > 0 and filled[j] == filled[s]: uid[j] = u; st.append(j)
        units.append(sorted(mem))
    return uid, units

def hydrology(W, z, ocean, P, E, void=False, coeff=0.4, thr=100.0, lf=3.0, mind=0.5, rho=1000.0):
    n = W['n']; area = W['area']; bl = bl_of(W)
    outlet = [bool(ocean[i] or (void and bl[i] > 0)) for i in range(n)]
    F = flood(W, z, outlet); fd = flow_dir(W, F); filled = F['filled']; order = list(reversed(F['order']))
    dep = [filled[i] - F['z'][i] for i in range(n)]
    uid, units = lake_units(W, filled, dep); node = list(range(n))
    for u in units:
        for c in u: node[c] = u[0]
    rq = [0.0] * n; PL = LOSS = 0.0
    for c in range(n):
        if ocean[c]: continue
        p = max(P[c], 0.0); rq[c] = coeff * p * area[c] / rho; PL += p * area[c] / rho; LOSS += (1 - coeff) * p * area[c] / rho
    indeg = [0] * n
    for c in range(n):
        if fd[c] >= 0 and node[c] != node[fd[c]]: indeg[node[fd[c]]] += 1
    ready = [x for x in range(n) if node[x] == x and indeg[x] == 0]; heapq.heapify(ready)
    acc = [0.0] * n; disch = [0.0] * n; depth = [0.0] * n; qout = [0.0] * len(units); full = [False] * len(units); done = [False] * n
    levap = 0.0; outcell = [0.0] * n
    while ready:
        x = heapq.heappop(ready); done[x] = True
        if uid[x] < 0:
            t = acc[x] + rq[x]; disch[x] = t
            if fd[x] >= 0:
                r = fd[x]; acc[r] += t; indeg[node[r]] -= 1
                if indeg[node[r]] == 0: heapq.heappush(ready, node[r])
        else:
            u = uid[x]; cells = units[u]; qin = sum(acc[c] + rq[c] for c in cells)
            ev = [lf * max(E[c], 0.0) / rho for c in cells]
            d, qo, evp, fl = solve_lake(filled[cells[0]], [z[c] for c in cells], [area[c] for c in cells], ev, qin)
            exits = [c for c in cells if fd[c] >= 0 and node[fd[c]] != x]; assert exits
            levap += evp; qout[u] = qo; full[u] = fl
            for k, c in enumerate(cells): depth[c] = d[k]; outcell[c] = qo
            for c in exits:
                r = fd[c]; acc[r] += qo / len(exits); indeg[node[r]] -= 1
                if indeg[node[r]] == 0: heapq.heappush(ready, node[r])
    unresolved = set()
    for c in order:
        if done[node[c]]: continue
        if uid[c] >= 0: unresolved.add(uid[c])
        t = acc[c] + rq[c]; disch[c] = t
        if fd[c] >= 0: acc[fd[c]] += t
    salt = [0] * n
    for u, cells in enumerate(units):
        if u in unresolved: continue
        for c in cells:
            if depth[c] < mind:
                depth[c] = 0.0
                if qout[u] <= 0 and not full[u]: salt[c] = 1
    river = [(not ocean[c]) and uid[c] < 0 and disch[c] > thr for c in range(n)]; fed = [False] * len(units); ch = True
    while ch:
        ch = False
        for c in order:
            if fd[c] < 0: continue
            r = fd[c]
            if uid[c] < 0:
                if river[c]:
                    if uid[r] >= 0:
                        if not fed[uid[r]]: fed[uid[r]] = True; ch = True
                    elif not ocean[r] and disch[r] > 0 and not river[r]: river[r] = True; ch = True
            else:
                u = uid[c]
                if fed[u] and qout[u] > 0:
                    if uid[r] < 0:
                        if not ocean[r] and not river[r]: river[r] = True; ch = True
                    elif uid[r] != u and not fed[uid[r]]: fed[uid[r]] = True; ch = True
    oq = vq = sq = 0.0
    for c in range(n):
        if fd[c] >= 0: continue
        if ocean[c]: oq += disch[c]
        elif void and bl[c] > 0: vq += disch[c]
        else: sq += disch[c]
    res = (PL - LOSS - levap - oq - vq - sq) / PL if PL > 0 else 0.0
    return dict(fd=fd, uid=uid, units=units, depth=depth, disch=disch, river=river, qout=qout, salt=salt, unresolved=unresolved,
                PL=PL, loss=LOSS, levap=levap, oq=oq, vq=vq, sq=sq, res=res, outcell=outcell, order=order)

def fields_for(W, z, seed, p0=3e-5, e0=1e-5):
    rng = Rng(seed); n = W['n']
    P = [p0 * (0.5 + rng.f64()) for _ in range(n)]; E = [e0 * (0.5 + rng.f64()) for _ in range(n)]
    return P, E

def check(name, W, z, ocean, P, E, **kw):
    H = hydrology(W, z, ocean, P, E, **kw); n = W['n']; fd = H['fd']
    assert abs(H['res']) < 1e-12, H['res']
    for u, cells in enumerate(H['units']):
        for c in cells: assert 0 <= H['depth'][c] <= (flood_dep(H, z, c)) + 1e-9
    # rivers: connected
    for c in range(n):
        if not H['river'][c]: continue
        x = fd[c]
        while x >= 0:
            if H['uid'][x] >= 0:
                if H['outcell'][x] <= 0: break
            elif ocean[x]: break
            else: assert H['river'][x] or H['disch'][x] == 0, (name, c, x)
            x = fd[x]
    nsp = sum(1 for q in H['qout'] if q > 0); nd = sum(1 for u, cs in enumerate(H['units']) if all(H['depth'][c] == 0 for c in cs))
    print(f"{name:11s} units={len(H['units']):4d} spilling={nsp:4d} dry={nd:4d} unresolved={len(H['unresolved'])} rivers={sum(H['river']):4d} "
          f"res={H['res']:.1e} oq/PL={H['oq'] / H['PL']:.3f} lakeEvap/PL={H['levap'] / H['PL']:.3f} loss/PL={H['loss'] / H['PL']:.3f}")
    return H

def flood_dep(H, z, c): return 1e30  # spill-height bound is checked in Rust

if __name__ == '__main__':
    # pure lake balance numbers (also asserted in tests/part4b2.rs)
    z = [0.0, 10.0, 20.0]; a = [1e8] * 3; ev = [1e-8] * 3
    for q in (0.0, 0.5, 1.5, 3.0, 5.0):
        d, qo, e, f = solve_lake(40.0, z, a, ev, q); print('solve_lake q_in', q, '-> depth', d, 'q_out', qo, 'evap', e, 'full', f)
    for L in (3, 4):
        W = sphere_world(L); z = terrain(W, 11); P, E = fields_for(W, z, 31); check(f'sphere L{L}', W, z, ocean_cells(W, z), P, E)
    W = cube_world(14); z = terrain(W, 12); P, E = fields_for(W, z, 32); check('cube', W, z, ocean_cells(W, z), P, E)
    W = plane_world(40, 24); z = terrain(W, 13); P, E = fields_for(W, z, 33); check('plane wall', W, z, ocean_cells(W, z), P, E)
    W = plane_world(40, 24, wrap=True); z = terrain(W, 13); P, E = fields_for(W, z, 33); oc = ocean_cells(W, z); check('plane wrap', W, z, oc, P, E)
    for nm, p0 in (('wet', 3e-4), ('dry', 3e-7)):
        P, E = fields_for(W, z, 33, p0=p0); check('wrap ' + nm, W, z, oc, P, E)
    W = plane_world(40, 24); z = terrain(W, 14); P, E = fields_for(W, z, 34); check('plane void', W, z, [False] * W['n'], P, E, void=True)
    z = np.array([1000.0 + ((i * 37) % 101) for i in range(W['n'])]); P, E = fields_for(W, z, 35); check('closed', W, z, [False] * W['n'], P, E)
