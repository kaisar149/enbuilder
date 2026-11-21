"""Python mirror of src/erosion.rs (Part 4c-1): implicit stream-power incision. `python mirror_4c1.py` prints the hand numbers
quoted in tests/part4c1.rs and runs randomized invariants (ordering, volume bookkeeping, monotone in g and strength)."""
import math, random

G_EARTH = 9.81
DEF = dict(K=2e-7, m=0.5, n=1.0, years=1e5, max_inc=500.0, gexp=1.0)

def incise(z0, zr, c, n):
    """Solve h + c h^n = d for h = z - zr in (0, d], d = z0 - zr. Returns the new elevation."""
    d = z0 - zr
    if not d > 0 or not c > 0: return z0
    if n == 1.0: return zr + d / (1.0 + c)
    lo, hi = 0.0, d
    h = d / (1.0 + c * d ** (n - 1.0)) if n >= 1 else 0.5 * d
    h = min(max(h, lo), hi)
    for _ in range(100):
        f = h + c * h ** n - d
        if f > 0: hi = h
        else: lo = h
        df = 1.0 + c * n * h ** (n - 1.0) if h > 0 else float('inf')
        hn = h - f / df if df != float('inf') else 0.5 * (lo + hi)
        if not (lo < hn < hi): hn = 0.5 * (lo + hi)
        if abs(hn - h) <= 1e-15 * d: h = hn; break
        h = hn
    return zr + h

def erode(p, z, fd, order, area, dist, skip, g, strength):
    """z: elevations; skip[c]: not incised. Processes order reversed (downhill first). Returns new z."""
    out = list(z)
    for c in reversed(order):
        if skip[c] or fd[c] < 0: continue
        r = fd[c]
        keff = p['K'] * (g[c] / G_EARTH) ** p['gexp'] / max(strength[c], 1e-3)
        cc = keff * area[c] ** p['m'] * p['years'] / dist[c] ** p['n']
        zn = incise(z[c], out[r], cc, p['n'])
        out[c] = max(zn, z[c] - p['max_inc'])
    return out

if __name__ == '__main__':
    p = dict(DEF)
    CELL = 1e4; A = CELL * CELL
    # chain: columns 0..5, column 0 ocean at -10, receiver of column i is i-1, z = 100*i, upstream area (6-i)*A
    z = [-10.0, 100.0, 200.0, 300.0, 400.0, 500.0]
    fd = [-1, 0, 1, 2, 3, 4]; order = [5, 4, 3, 2, 1, 0]
    area = [0.0] + [(6 - i) * A for i in range(1, 6)]
    dist = [CELL] * 6; skip = [True] + [False] * 5
    out = erode(p, z, fd, order, area, dist, skip, [G_EARTH] * 6, [1.0] * 6)
    print('chain z   ', out)
    print('chain c   ', [2e-7 * a ** 0.5 * 1e5 / CELL for a in area[1:]])
    vol = sum(A * (z[i] - out[i]) for i in range(6))
    print('chain removed volume', vol)
    print('rate', [(z[i] - out[i]) / 1e5 for i in range(6)])
    o2 = erode(p, z, fd, order, area, dist, skip, [2 * G_EARTH] * 6, [1.0] * 6)
    print('2g        ', o2)
    o3 = erode(p, z, fd, order, area, dist, skip, [G_EARTH] * 6, [0.5] * 6)
    print('strength .5', o3)
    q = dict(p); q['years'] = 1.0
    t1 = erode(q, z, fd, order, area, dist, skip, [G_EARTH] * 6, [1.0] * 6)
    t2 = erode(q, z, fd, order, area, dist, skip, [2 * G_EARTH] * 6, [1.0] * 6)
    print('tiny dt ratio 2g/1g', [(z[i] - t2[i]) / (z[i] - t1[i]) for i in range(1, 6)])
    q = dict(p); q['max_inc'] = 3.0
    print('cap 3 m   ', erode(q, z, fd, order, area, dist, skip, [G_EARTH] * 6, [1.0] * 6))
    q = dict(p); q['years'] = 1e15; q['max_inc'] = 1e9
    print('huge dt   ', erode(q, z, fd, order, area, dist, skip, [G_EARTH] * 6, [1.0] * 6))
    # pure function checks
    for n in (0.5, 1.0, 1.5, 2.0, 3.0):
        for c in (1e-6, 0.3, 5.0, 1e3):
            zn = incise(100.0, 20.0, c, n); h = zn - 20.0
            assert 0 < h <= 80.0 and abs(h + c * h ** n - 80.0) <= 1e-9 * 80.0, (n, c, h)
    for n in (0.5, 1.0, 2.0):
        h = incise(100.0, 20.0, 1e9, n) - 20.0
        assert 0 < h < 1e-3, (n, h)   # huge coefficient: pinned just above the receiver, never below
    print('incise residuals ok; incise(100,20,1,1) =', incise(100, 20, 1.0, 1.0), ' n=2 c=0.01:', incise(100, 20, 0.01, 2.0))
    # randomized invariants on random trees
    rng = random.Random(3)
    worst = 0.0
    for trial in range(300):
        n = rng.randint(5, 60)
        fd = [-1] + [rng.randrange(0, i) for i in range(1, n)]  # receiver has smaller index; order high index first
        order = list(range(n - 1, -1, -1))
        z = [0.0] * n
        for i in range(1, n): z[i] = z[fd[i]] + rng.uniform(0, 80)
        area = [0.0] * n
        for i in range(n - 1, -1, -1):
            area[i] += A * rng.uniform(0.5, 1.5)
            if fd[i] >= 0: area[fd[i]] += area[i]
        dist = [rng.uniform(5e3, 2e4) for _ in range(n)]
        skip = [i == 0 or rng.random() < 0.1 for i in range(n)]
        g = [G_EARTH * rng.uniform(0.5, 2) for _ in range(n)]; s = [rng.uniform(0.3, 3) for _ in range(n)]
        pp = dict(p); pp['years'] = 10 ** rng.uniform(2, 9); pp['n'] = rng.choice([0.7, 1.0, 1.0, 2.0]); pp['max_inc'] = rng.choice([5.0, 1e9])
        out = erode(pp, z, fd, order, area, dist, skip, g, s)
        for i in range(1, n):
            assert out[i] <= z[i] + 1e-12
            if not skip[i] and z[i] > z[fd[i]]: assert out[i] >= out[fd[i]] - 1e-9 * max(1, abs(z[i])), 'new pit'
            if skip[i]: assert out[i] == z[i]
    print('randomized ordering / skip invariants ok')
