"""Hand-built cases of tests/part4b1.rs, checked with the python mirror (expected numbers quoted in the Rust tests)."""
import numpy as np
from mirror_4a2e import plane_world
from mirror_4b1 import flood, flow_dir, accumulate, ocean_cells

def go(W, z, ocean=None, void=False):
    n = W['n']; ocean = ocean_cells(W, z) if ocean is None else ocean
    bl = W['bl']; outlet = [bool(ocean[i] or (void and bl[i] > 0)) for i in range(n)]
    F = flood(W, z, outlet); fd = flow_dir(W, F); order = list(reversed(F['order']))
    return F, fd, accumulate(W, fd, order, [not o for o in ocean]), ocean

def idx(nx, i, j): return j * nx + i

# --- bowl with a west spill channel (9 x 7, ocean column i = 0)
nx, ny = 9, 7; W = plane_world(nx, ny); z = np.full(W['n'], 100.0)
for j in range(ny): z[idx(nx, 0, j)] = -10.0
z[idx(nx, 4, 3)] = 20; z[idx(nx, 5, 3)] = 20; z[idx(nx, 3, 3)] = 60; z[idx(nx, 2, 3)] = 40; z[idx(nx, 1, 3)] = 30
F, fd, acc, ocean = go(W, z)
print('bowl filled', [F['filled'][idx(nx, i, 3)] for i in range(nx)])
print('bowl fd   ', [fd[idx(nx, i, 3)] for i in range(nx)], ' expect cells', [idx(nx, i, 3) for i in range(nx)])
print('dep cells ', [i for i in range(W['n']) if F['filled'][i] != z[i]])
print('fd (4,2),(4,4),(5,2),(6,3):', fd[idx(nx, 4, 2)], fd[idx(nx, 4, 4)], fd[idx(nx, 5, 2)], fd[idx(nx, 6, 3)], ' ids', idx(nx, 4, 3), idx(nx, 5, 3))
print('ocean sum', sum(acc[idx(nx, 0, j)] for j in range(ny)), 'land', 8 * 7 * 1e8, 'ocean n', sum(ocean))

# --- flat plateau (20 x 10), path length to ocean == i
nx, ny = 20, 10; W = plane_world(nx, ny); z = np.full(W['n'], 100.0)
for j in range(ny): z[idx(nx, 0, j)] = -10.0
F, fd, acc, ocean = go(W, z)
bad = 0
for j in range(ny):
    for i in range(1, nx):
        x = idx(nx, i, j); k = 0
        while fd[x] >= 0: x = fd[x]; k += 1
        bad += (k != i)
print('plateau path-length mismatches', bad, 'dep cells', sum(F['filled'][i] != z[i] for i in range(W['n'])))

# --- sub-sea-level pit that is NOT ocean (1 cell < 1% of 200 cells)
z2 = z.copy(); pit = idx(nx, 10, 5); z2[pit] = -50.0
F, fd, acc, ocean = go(W, z2)
print('pit ocean?', ocean[pit], 'filled', F['filled'][pit], 'depth', F['filled'][pit] - z2[pit], 'fd', fd[pit], 'expect', idx(nx, 9, 5))

# --- tilted plane: every land cell drains to its west neighbour
z3 = np.array([(50.0 * (c % nx) - 40.0) if c % nx == 0 else 10.0 + 50.0 * (c % nx) for c in range(W['n'])])
F, fd, acc, ocean = go(W, z3)
print('slope fd ok', all(fd[c] == c - 1 for c in range(W['n']) if c % nx >= 1), 'dep', sum(F['filled'][i] != z3[i] for i in range(W['n'])))

# --- void plane 30 x 10
nx, ny = 30, 10; W = plane_world(nx, ny); z4 = np.array([100.0 + 5.0 * (c % nx) + 0.3 * (c // nx) for c in range(W['n'])])
F, fd, acc, ocean = go(W, z4, ocean=[False] * W['n'], void=True)
roots = [i for i in range(W['n']) if fd[i] < 0]
print('void roots', len(roots), 'all on boundary', all(W['bl'][i] > 0 for i in roots), 'sum acc roots', sum(acc[i] for i in roots), 'total', 3e10)
