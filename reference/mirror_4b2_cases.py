"""Hand-built cases of tests/part4b2.rs, checked with the python mirror (expected numbers quoted in the Rust tests).
   python mirror_4b2_cases.py   (needs mirror_4b2.py, mirror_4b1.py, mirror_4a2e.py)"""
import numpy as np
from mirror_4a2e import plane_world
from mirror_4b2 import hydrology

def idx(nx, i, j): return j * nx + i
def world(nx, ny, cells, base=100.0):
    W = plane_world(nx, ny); z = np.full(W['n'], base)
    for j in range(ny): z[idx(nx, 0, j)] = -10.0
    for (i, j), v in cells.items(): z[idx(nx, i, j)] = v
    return W, z
def ocean_col(W, nx): return [c % nx == 0 for c in range(W['n'])]
def run(W, z, p, e, **kw):
    n = W['n']; oc = ocean_col(W, W['nx'])
    return hydrology(W, z, oc, [p] * n, [e] * n, **kw)

# A: bowl (4,3)=20,(5,3)=30 in a 9x7 world, channel (3,3)=60,(2,3)=40,(1,3)=30; ocean column 0. rq/cell = 0.4*P*1e8/1e3
nx, ny = 9, 7
W, z = world(nx, ny, {(4, 3): 20, (5, 3): 30, (3, 3): 60, (2, 3): 40, (1, 3): 30})
H = run(W, z, 1e-4, 1e-5)
b = idx(nx, 4, 3); print('A wet  units', H['units'], 'depth', [H['depth'][c] for c in H['units'][0]], 'qout', H['qout'], 'levap', H['levap'],
    'disch(3,3)', H['disch'][idx(nx, 3, 3)], 'disch(1,3)', H['disch'][idx(nx, 1, 3)], 'res', H['res'])
cnt = None
# number of land cells draining into the bowl = qin / rq ; rq = 4.0 for P=1e-4
qin = H['qout'][0] + H['levap']; print('   qin', qin, 'cells', qin / 4.0, ' PL', H['PL'], 'oq', H['oq'], 'loss', H['loss'])
for P, name in ((4.5e-6, 'partial'), (0.0, 'norain')):
    H = run(W, z, P, 1e-5); c = H['units'][0]
    print('A', name, 'depth', [H['depth'][k] for k in c], 'salt', [H['salt'][k] for k in c], 'qout', H['qout'], 'levap', H['levap'], 'res', H['res'], 'ocean', H['oq'], 'PL', H['PL'])
# find P giving partial fill: qin in [3,6): rq=0.4*P*1e5 per cell
for P in (2e-5, 3e-5, 5e-5):
    H = run(W, z, P, 1e-5); c = H['units'][0]; qin = H['levap'] + H['qout'][0]
    print('   P', P, 'qin', qin, 'depth', [round(H['depth'][k], 3) for k in c], 'salt', [H['salt'][k] for k in c], 'qout', H['qout'][0])

# B: cascade (ridge 50 above A's rim 30)  A=(2,3)=20, ridge (3,3)=50, B=(4,3)=20, rim (1,3)=30
W, z = world(nx, ny, {(1, 3): 30, (2, 3): 20, (3, 3): 50, (4, 3): 20})
H = run(W, z, 1e-4, 1e-5); print('B cascade units', H['units'], 'depth', [[H['depth'][k] for k in c] for c in H['units']], 'qout', H['qout'], 'res', H['res'],
    'river?', [H['river'][k] for k in range(W['n']) if H['river'][k]][:3])
# C: merge (ridge 25 below rim 30)
W, z = world(nx, ny, {(1, 3): 30, (2, 3): 20, (3, 3): 25, (4, 3): 20})
H = run(W, z, 1e-4, 1e-5); print('C merge units', H['units'], 'depth', [[H['depth'][k] for k in c] for c in H['units']], 'qout', H['qout'], 'res', H['res'])

# D: tilted plane 20x10: discharge(i,j) = rq*(nx-i); river iff > thr
nx2, ny2 = 20, 10; W = plane_world(nx2, ny2)
z = np.array([(-40.0) if c % nx2 == 0 else 10.0 + 50.0 * (c % nx2) for c in range(W['n'])])
H = run(W, z, 1e-4, 1e-5, thr=40.0)
print('D slope discharge row3', [H['disch'][idx(nx2, i, 3)] for i in (0, 1, 9, 10, 19)], 'rivers', sorted({c % nx2 for c in range(W['n']) if H['river'][c]}), 'n', sum(H['river']), 'units', len(H['units']), 'res', H['res'])

print('--- tuned values quoted in tests')
W, z = world(nx, ny, {(4, 3): 20, (5, 3): 30, (3, 3): 60, (2, 3): 40, (1, 3): 30})
H = run(W, z, 3e-6, 1e-5); c = H['units'][0]
print('A partial P=3e-6 depth', [round(float(H['depth'][k]), 9) for k in c], 'salt', [H['salt'][k] for k in c], 'qout', H['qout'], 'levap', H['levap'], 'PL', H['PL'], 'oq', H['oq'], 'res', H['res'])
W2 = plane_world(nx2, ny2)
z = np.array([(-40.0) if c % nx2 == 0 else 10.0 + 50.0 * (c % nx2) for c in range(W2['n'])])
H = run(W2, z, 1e-4, 1e-5, thr=42.0)
print('D thr=42 river columns', sorted({c % nx2 for c in range(W2['n']) if H['river'][c]}), 'n', sum(H['river']))
from mirror_4b2 import fields_for
from mirror_4b1 import terrain, ocean_cells
W3 = plane_world(40, 24, wrap=True); z3 = terrain(W3, 13); P, E = fields_for(W3, z3, 33); oc = ocean_cells(W3, z3)
H = hydrology(W3, z3, oc, P, E, lf=0.0); print('wrap lf=0: oq/PL', H['oq'] / H['PL'], 'levap', H['levap'], 'sinks', H['sq'], 'units', len(H['units']), 'spilling', sum(q > 0 for q in H['qout']))
H = hydrology(W3, z3, oc, P, E, coeff=0.0); print('coeff=0: oq', H['oq'], 'rivers', sum(H['river']), 'PL', H['PL'])
H = hydrology(W3, z3, oc, P, E, coeff=1.0); print('coeff=1: loss', H['loss'], 'res', H['res'])
P, E = fields_for(W3, z3, 33, p0=3e-4); H = hydrology(W3, z3, oc, P, E); print('wet spilling', sum(q > 0 for q in H['qout']), len(H['units']), 'unresolved', len(H['unresolved']))
P, E = fields_for(W3, z3, 33, p0=3e-7); H = hydrology(W3, z3, oc, P, E); print('dry spilling', sum(q > 0 for q in H['qout']), len(H['units']), 'unresolved', len(H['unresolved']), 'salt', sum(H['salt']))
