"""Python mirror of src/sediment.rs (Part 4c-2): sediment transport, deposition (floodplains, deltas, lake trapping) and thermal
erosion on the eroded surface. `python mirror_4c2.py` runs the invariants and prints the hand numbers quoted in tests/part4c2.rs.
Needs mirror_4b1.py / mirror_4a2e.py (same folder), numpy."""
import heapq, math, sys
import numpy as np
from mirror_4a2e import sphere_world, plane_world, cube_world
import mirror_4b1 as m41
from mirror_4c1 import incise, G_EARTH

DEF = dict(Kc=2.0e5, a_exp=1.0, years=1.0e5, q_per_area=1.0e-8, deposit_fraction=0.5, talus_scale=1.0, max_slope=0.05,
           thermal_iters=400, thermal_tol_m=1e-6, sea_level=0.0, floodplain_slope=0.005)

# ----------------------------------------------------------------------------- routing glue
def route(W, z, ocean, void=False):
    n = W['n']; bl = m41.bl_of(W)
    outlet = [bool(ocean[i] or (void and bl[i] > 0)) for i in range(n)]
    F = m41.flood(W, z, outlet); fd = m41.flow_dir(W, F); order = list(reversed(F['order'])); land = [not o for o in ocean]
    acc = m41.accumulate(W, fd, order, land)
    dep = [F['filled'][i] - F['z'][i] for i in range(n)]
    return dict(filled=F['filled'], fd=fd, order=order, area=acc, dep=dep, ocean=list(ocean), void=void)

# ----------------------------------------------------------------------------- pure pieces
def lake_units(W, filled, dep):
    n = W['n']; uid = [-1] * n; units = []
    for s in range(n):
        if uid[s] >= 0 or not dep[s] > 0: continue
        u = len(units); uid[s] = u; stack = [s]; mem = []
        while stack:
            c = stack.pop(); mem.append(c)
            for j in W['adj'][c]:
                if uid[j] < 0 and dep[j] > 0 and filled[j] == filled[s]: uid[j] = u; stack.append(j)
        units.append(sorted(mem))
    return uid, units

def fill_basin(floors, areas, top, volume):
    """Water-filling: raise the lowest floors to a common level L <= top holding `volume` m^3. Returns (L, trapped)."""
    m = len(floors); idx = sorted(range(m), key=lambda k: (floors[k], k))
    cap = sum(areas[k] * max(top - floors[k], 0.0) for k in range(m))
    if not volume > 0: return (min(floors) if m else top), 0.0
    if volume >= cap: return top, cap
    acc = 0.0; asum = 0.0; cur = floors[idx[0]]
    for t, k in enumerate(idx):
        # raise from cur to floors[k] with the area of the cells below
        if t > 0:
            nxt = floors[k]
            need = asum * (nxt - cur)
            if acc + need >= volume:
                return cur + (volume - acc) / asum, volume
            acc += need; cur = nxt
        asum += areas[k]
    # all floors passed, level between last floor and top
    return min(cur + (volume - acc) / asum, top), volume

def thermal_relax(W, z, g, strength, p):
    """Pairwise area-weighted projection (Gauss-Seidel over edges, i < j in index order), volume conserving. Returns (z, iters, max_excess)."""
    n = W['n']; e = list(z); area = W['area']; smax = p['talus_scale'] * p['max_slope']
    lim = {}
    for i in range(n):
        for k, j in enumerate(W['adj'][i]):
            if j > i:
                gp = 0.5 * (g[i] + g[j]); sp = 0.5 * (strength[i] + strength[j])
                lim[(i, j)] = smax * (G_EARTH / gp) * sp * W['ed'][i][k]
    keys = sorted(lim)
    it = 0; mx = 0.0
    while True:
        mx = 0.0
        for (i, j) in keys:
            d = e[i] - e[j]; ex = abs(d) - lim[(i, j)]
            if ex > 0.0:
                mx = max(mx, ex)
        if mx <= p['thermal_tol_m'] or it >= p['thermal_iters']: break
        for (i, j) in keys:
            d = e[i] - e[j]; ex = abs(d) - lim[(i, j)]
            if ex > 0.0:
                v = ex / (1.0 / area[i] + 1.0 / area[j])
                if d > 0: e[i] -= v / area[i]; e[j] += v / area[j]
                else: e[j] -= v / area[j]; e[i] += v / area[i]
        it += 1
    return e, it, mx, lim

# ----------------------------------------------------------------------------- the module
def sediment_step(W, z, step_e, R, strength, g, p, discharge=None):
    n = W['n']; area = W['area']; fd = R['fd']; filled = R['filled']; dep = R['dep']; ocean = R['ocean']
    years = p['years']
    z1 = [z[c] + step_e[c] for c in range(n)]
    src = [area[c] * max(-step_e[c], 0.0) for c in range(n)]
    uid, units = lake_units(W, filled, dep)
    dissolved = set()
    def build():
        node = [(n + uid[c]) if (uid[c] >= 0 and uid[c] not in dissolved) else c for c in range(n)]
        return node
    while True:
        node = build(); indeg = {}
        nodes = set(node)
        for v in nodes: indeg[v] = 0
        for c in range(n):
            if fd[c] >= 0 and node[fd[c]] != node[c]: indeg[node[fd[c]]] += 1
        members = {}
        for c in range(n): members.setdefault(node[c], []).append(c)
        heap = [v for v in nodes if indeg[v] == 0]; heapq.heapify(heap); seq = []
        while heap:
            v = heapq.heappop(heap); seq.append(v)
            for c in members[v]:
                if fd[c] >= 0 and node[fd[c]] != v:
                    w = node[fd[c]]; indeg[w] -= 1
                    if indeg[w] == 0: heapq.heappush(heap, w)
        if len(seq) == len(nodes): break
        left = [v for v in nodes if v not in set(seq) and v >= n]
        assert left, 'cell graph cannot be cyclic'
        for v in left: dissolved.add(v - n)
    lake = [(uid[c] if (uid[c] >= 0 and uid[c] not in dissolved) else -1) for c in range(n)]
    load = list(src); hmin = [math.inf] * n
    dh = [0.0] * n; cap_out = [0.0] * n; flux = [0.0] * n; slope = [0.0] * n
    B = dict(eroded=sum(src), land=0.0, delta=0.0, lakes=0.0, ocean=0.0, void=0.0, sink=0.0)
    def dist(c, r): return W['ed'][c][W['adj'][c].index(r)]
    def heff_pass(c, h):
        r = fd[c]
        if r >= 0 and h < hmin[r]: hmin[r] = h
    for v in seq:
        if v >= n:
            mem = members[v]; mset = set(mem); L = sum(load[c] for c in mem)
            floors = [z1[c] for c in mem]; areas = [area[c] for c in mem]; top = filled[mem[0]]
            lvl, trapped = fill_basin(floors, areas, top, L)
            for c in mem:
                if trapped > 0: dh[c] = max(lvl - z1[c], 0.0)
            B['lakes'] += trapped
            outv = L - trapped
            exits = [c for c in mem if fd[c] >= 0 and fd[c] not in mset]
            if exits:
                for c in exits:
                    load[fd[c]] += outv / len(exits); flux[c] = outv / len(exits) / years
                    heff_pass(c, filled[c])
            else:
                B['sink'] += outv
            continue
        c = v; Lc = load[c]
        if ocean[c]:
            ceil = min(hmin[c], p['sea_level']) - z1[c]
            hr = max(ceil, 0.0)
            d = min(p['deposit_fraction'] * Lc, hr * area[c])
            dh[c] = d / area[c]; B['delta'] += d; B['ocean'] += Lc - d; flux[c] = (Lc - d) / years
            continue
        if fd[c] < 0:
            if R['void'] and m41.bl_of(W)[c] > 0: B['void'] += Lc
            else: B['sink'] += Lc
            flux[c] = Lc / years
            continue
        r = fd[c]
        base = filled[r] if lake[r] >= 0 else z1[r]
        S = max(z1[c] - base, 0.0) / dist(c, r); slope[c] = S
        Q = discharge[c] if discharge is not None else p['q_per_area'] * R['area'][c]
        cap = p['Kc'] * (Q ** p['a_exp']) * S; cap_out[c] = cap
        excess = max(Lc - cap * years, 0.0)
        d = p['deposit_fraction'] * excess
        if hmin[c] < math.inf: d = min(d, max(hmin[c] - z1[c], 0.0) * area[c])
        dh[c] = d / area[c]; B['land'] += d
        outv = Lc - d; load[r] += outv; flux[c] = outv / years
        heff_pass(c, z1[c] + dh[c])
    z2 = [z1[c] + dh[c] for c in range(n)]
    z3, it, mx, lim = thermal_relax(W, z2, g, strength, p) if p['thermal_iters'] > 0 else (z2, 0, 0.0, None)
    tstep = [z3[c] - z2[c] for c in range(n)]
    tvol = sum(area[c] * tstep[c] for c in range(n))
    delta_mask = [0] * n; fp_mask = [0] * n
    for c in range(n):
        if dh[c] <= 0: continue
        if ocean[c] or (fd[c] >= 0 and ocean[fd[c]]): delta_mask[c] = 1
        elif lake[c] < 0 and slope[c] <= p['floodplain_slope']: fp_mask[c] = 1
    return dict(z1=z1, z2=z2, z3=z3, dh=dh, tstep=tstep, tvol=tvol, B=B, flux=flux, cap=cap_out, slope=slope, delta=delta_mask, fp=fp_mask,
                lake=lake, n_unresolved=len(dissolved), iters=it, excess=mx, units=units, load=load)

def budget_residual(B):
    s = B['land'] + B['delta'] + B['lakes'] + B['ocean'] + B['void'] + B['sink']
    return abs(B['eroded'] - s) / max(B['eroded'], 1e-300)

# ----------------------------------------------------------------------------- helpers for worlds
def full_erosion(W, z, R, strength, g, e):
    """Stream power step as in 4c-1 (mirror_4c1.erode semantics) on a routed world; returns erosion_step."""
    n = W['n']; fd = R['fd']; order = R['order']; out = list(z)
    for c in reversed(order):
        if R['ocean'][c] or R['dep'][c] > 0 or fd[c] < 0: continue
        r = fd[c]; dist = W['ed'][c][W['adj'][c].index(r)]
        keff = e['K'] * (g[c] / G_EARTH) ** e['gexp'] / max(strength[c], 1e-3)
        coef = keff * max(R['area'][c], 0.0) ** e['m'] * e['years'] / dist ** e['n']
        base = R['filled'][r] if R['dep'][r] > 0 else out[r]
        cut = incise(z[c], base, coef, e['n'])
        out[c] = max(cut, z[c] - e['max_inc'])
    return [out[c] - z[c] for c in range(n)]

EROS = dict(K=2e-7, m=0.5, n=1.0, years=1e5, max_inc=500.0, gexp=1.0)

def world_terrain(W, seed): return m41.terrain(W, seed)
