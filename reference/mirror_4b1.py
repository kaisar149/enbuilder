"""Numpy/pure-python mirror of Part 4b-1 (src/flow_routing.rs): same priority-flood key (filled, hops, idx), same steepest-descent rule,
same accumulation. Checks it against an independent Planchon-Darboux fill and prints the numbers used for tests/part4b1.rs.
Run from this folder (needs mirror_4a2e.py, mirror_gyres.py, mir.py, numpy, scipy):   python mirror_4b1.py
"""
import heapq, sys, numpy as np
from mirror_4a2e import sphere_world, plane_world, cube_world

M64 = (1 << 64) - 1
class Rng:                                   # SplitMix64, identical to src/rng.rs
    def __init__(s, seed): s.s = seed & M64
    def u64(s):
        s.s = (s.s + 0x9E3779B97F4A7C15) & M64; z = s.s
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & M64
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & M64
        return z ^ (z >> 31)
    def f64(s): return (s.u64() >> 11) / float(1 << 53)

def bl_of(W): return W.get('bl', np.zeros(W['n']))

def terrain(W, seed):
    """Same as `terrain()` in tests/part4b1.rs: east-west ramp plus 300 m of per-cell jitter (index order)."""
    C = np.asarray(W['C']); n = W['n']; mid = C.mean(axis=0)
    ext = max(1.0, max(np.linalg.norm(C[i] - mid) for i in range(n)))
    rng = Rng(seed); z = np.zeros(n)
    for i in range(n):
        z[i] = 800.0 * ((C[i][0] - mid[0]) / ext) - 150.0 + 300.0 * rng.f64()
    return z

def ocean_cells(W, z, sea=0.0, frac=0.01):
    n = W['n']; below = z < sea; seen = [False] * n; ocean = [False] * n; thr = frac * float(np.sum(W['area']))
    for s in range(n):
        if seen[s] or not below[s]: continue
        seen[s] = True; q = [s]; mem = []
        while q:
            i = q.pop(); mem.append(i)
            for j in W['adj'][i]:
                if not seen[j] and below[j]: seen[j] = True; q.append(j)
        if sum(W['area'][i] for i in mem) >= thr:
            for i in mem: ocean[i] = True
    return ocean

def flood(W, z, outlet):
    n = W['n']; z = [float(v) + 0.0 for v in z]; filled = list(z); hops = [0] * n; parent = [-1] * n
    visited = [False] * n; root = [False] * n; sinks = []; order = []; heap = []
    for i in range(n):
        if outlet[i]: visited[i] = True; root[i] = True; heapq.heappush(heap, (z[i], 0, i))
    byh = None; cur = 0
    while True:
        while heap:
            h, hp, c = heapq.heappop(heap); order.append(c); fc = filled[c]; hc = hops[c]
            for j in W['adj'][c]:
                if visited[j]: continue
                visited[j] = True; parent[j] = c
                if z[j] <= fc: filled[j] = fc; hops[j] = hc + 1
                else: filled[j] = z[j]; hops[j] = 0
                heapq.heappush(heap, (filled[j], hops[j], j))
        if len(order) == n: break
        if byh is None: byh = sorted(range(n), key=lambda i: (z[i], i))
        while visited[byh[cur]]: cur += 1
        s = byh[cur]; visited[s] = True; root[s] = True; sinks.append(s); heapq.heappush(heap, (z[s], 0, s))
    return dict(filled=filled, parent=parent, order=order, root=root, sinks=sinks, z=z)

def flow_dir(W, F):
    n = W['n']; fd = [-1] * n
    for c in range(n):
        if F['root'][c]: continue
        fc = F['filled'][c]; best = -1; bs = 0.0
        for k, j in enumerate(W['adj'][c]):
            fj = F['filled'][j]
            if fj < fc:
                s = (fc - fj) / W['ed'][c][k]
                if best < 0 or s > bs: best = j; bs = s
        fd[c] = best if best >= 0 else F['parent'][c]
    return fd

def accumulate(W, fd, order, land):
    acc = [W['area'][i] if land[i] else 0.0 for i in range(W['n'])]
    for c in order:
        if fd[c] >= 0: acc[fd[c]] += acc[c]
    return acc

def planchon_darboux(W, z, root):
    """Independent iterative fill (no epsilon): W = z on roots, +inf elsewhere, relax until stable."""
    n = W['n']; w = [z[i] if root[i] else float('inf') for i in range(n)]; changed = True; sweeps = 0
    while changed:
        changed = False; sweeps += 1
        idx = range(n) if sweeps % 2 else range(n - 1, -1, -1)
        for c in idx:
            if root[c] or w[c] <= z[c]: continue
            for j in W['adj'][c]:
                if z[c] >= w[j]: w[c] = z[c]; changed = True; break
                if w[c] > w[j]: w[c] = w[j]; changed = True
    return w, sweeps

def run(name, W, z, ocean, void=False, void_outflow=True):
    n = W['n']; bl = bl_of(W)
    outlet = [bool(ocean[i] or (void and void_outflow and bl[i] > 0)) for i in range(n)]
    F = flood(W, z, outlet); fd = flow_dir(W, F); order = list(reversed(F['order'])); land = [not o for o in ocean]
    acc = accumulate(W, fd, order, land)
    # invariants
    pos = {c: k for k, c in enumerate(order)}
    assert sorted(order) == list(range(n))
    for c in range(n):
        if fd[c] >= 0:
            assert fd[c] in W['adj'][c] and pos[c] < pos[fd[c]] and F['filled'][fd[c]] <= F['filled'][c]
    maxlen = 0
    for c in range(n):
        k = 0; x = c
        while fd[x] >= 0: x = fd[x]; k += 1; assert k <= n
        assert F['root'][x]; maxlen = max(maxlen, k)
    roots = [i for i in range(n) if F['root'][i]]
    tot_land = sum(W['area'][i] for i in range(n) if land[i]); tot_roots = sum(acc[i] for i in roots)
    assert abs(tot_land - tot_roots) < 1e-9 * tot_land, (tot_land, tot_roots)
    pd, sweeps = planchon_darboux(W, F['z'], F['root'])
    err = max(abs(pd[i] - F['filled'][i]) for i in range(n))
    dep = [F['filled'][i] - F['z'][i] for i in range(n)]
    print(f"{name:12s} n={n:5d} outlets={sum(outlet):4d} sinks={len(F['sinks'])} dep_cells={sum(d > 0 for d in dep):4d} max_dep={max(dep):8.2f} "
          f"maxpath={maxlen:3d} PD_sweeps={sweeps:3d} |filled-PD|={err:.1e} land_frac={sum(land) / n:.2f}")
    assert err == 0.0
    return F, fd, acc

if __name__ == '__main__':
    for L in (3, 4):
        W = sphere_world(L); z = terrain(W, 11); run(f'sphere L{L}', W, z, ocean_cells(W, z))
    W = cube_world(14); z = terrain(W, 12); run('cube 14', W, z, ocean_cells(W, z))
    for nm, wrap in (('plane wall', False), ('plane wrap', True)):
        W = plane_world(40, 24, wrap=wrap); z = terrain(W, 13); run(nm, W, z, ocean_cells(W, z))
    W = plane_world(40, 24); z = terrain(W, 14); run('plane void', W, z, [False] * W['n'], void=True)
    # all-land walled plane: one terminal sink at the lowest cell
    z = np.array([(1000.0 + 0.1 * i) for i in range(W['n'])]); z[500] = 50.0
    F, fd, acc = run('plane wall-L', W, z, [False] * W['n']); print('  sink', F['sinks'], 'acc at sink', acc[F['sinks'][0]], 'total', sum(W['area']))
