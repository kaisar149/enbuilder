import math
from mirror_4c2 import *
np.set_printoptions(precision=10)
print('--- fill_basin')
for V in (0, 10, 40, 41, 100, 150):
    print(V, fill_basin([10, 10, 30, 50], [1, 1, 1, 1], 50.0, V))
print(fill_basin([5.0, 8.0], [2.0, 3.0], 20.0, 1e9))
print('--- chain (manual fields)')
W = plane_world(6, 3); n = W['n']
def chain(Kc, df=0.5, steps=(0, -5, -4, -3, -2, -1), years=1e5, g=G_EARTH, z=(-10, 100, 200, 300, 400, 500), thermal=0, ms=0.05):
    zz = [1000.0]*n; fd = [-1]*n; oc = [True]*n; area = [0.0]*n; es = [0.0]*n; filled = [1000.0]*n
    for i in range(6):
        c = 6 + i; zz[c] = float(z[i]); filled[c] = zz[c]; es[c] = steps[i]
        if i >= 1: fd[c] = c - 1; oc[c] = False; area[c] = (6 - i) * 1e8
    R = dict(filled=filled, fd=fd, order=None, area=area, dep=[0.0]*n, ocean=oc, void=False)
    p = dict(DEF, Kc=Kc, deposit_fraction=df, thermal_iters=thermal, max_slope=ms, years=years)
    return sediment_step(W, zz, es, R, [1.0]*n, [g]*n, p), zz
for Kc in (1e9, 2e4, 4e3, 0.0):
    o, zz = chain(Kc)
    B = o['B']
    print(f"Kc={Kc:g} eroded={B['eroded']!r} land={B['land']!r} delta={B['delta']!r} ocean={B['ocean']!r} resid={budget_residual(B):.1e}")
    print('  dh', [round(o['dh'][6+i], 10) for i in range(6)], ' ocean-col dh', o['dh'][6], ' S', [round(o['slope'][6+i], 6) for i in range(6)])
    print('  cap/yr', [o['cap'][6+i] for i in range(6)], ' flux/yr', [o['flux'][6+i] for i in range(6)])
    print('  delta', [i for i in range(n) if o['delta'][i]], 'fp', [i for i in range(n) if o['fp'][i]])
print('--- chain gravity 2 (thermal only, steep chain)')
print('--- thermal hand case')
W3 = plane_world(7, 3); n3 = W3['n']; z3 = [0.0]*n3; z3[1*7+3] = 1000.0
for gs, sts in ((1.0, 1.0), (2.0, 1.0), (1.0, 0.5)):
    p = dict(DEF, thermal_iters=20000, max_slope=0.05, thermal_tol_m=1e-9)
    e, it, mx, lim = thermal_relax(W3, z3, [G_EARTH*gs]*n3, [sts]*n3, p)
    vol = sum(W3['area'][c]*e[c] for c in range(n3))
    print(f"g x{gs} strength {sts}: iters={it} max_excess={mx:.2e} peak={e[10]!r} row1={[round(e[7+i], 6) for i in range(7)]} vol={vol!r}")
print('--- bowl')
NXB, NYB = 9, 7
def bowl_world():
    W = plane_world(NXB, NYB); n = W['n']; z = [0.0]*n
    for j in range(NYB):
        for i in range(NXB):
            z[j*NXB+i] = -10.0 if i == 0 else 100.0 + 10.0*i + 3.0*abs(j-3)
    def idx(i, j): return j*NXB+i
    for (i, j), v in (((4, 3), 20.0), ((5, 3), 20.0), ((3, 3), 60.0), ((2, 3), 40.0), ((1, 3), 30.0)): z[idx(i, j)] = v
    return W, z
W, z = bowl_world(); n = W['n']; oc = [v < 0 for v in z]; R = route(W, z, oc)
print('dep cells', [c for c in range(n) if R['dep'][c] > 0], 'filled', R['filled'][3*NXB+4], 'lake basin vol', 2*1e8*(60-20))
def bowl(Kc, src_cells, amount, df=0.5, years=1e5):
    es = [0.0]*n
    for c in src_cells: es[c] = -amount
    p = dict(DEF, Kc=Kc, deposit_fraction=df, thermal_iters=0, years=years)
    return sediment_step(W, z, es, R, [1.0]*n, [G_EARTH]*n, p), es
up = [j*NXB+i for j in range(NYB) for i in (6, 7, 8)]    # 21 cells east of the bowl
for amount in (5.0, 100.0):
    o, es = bowl(1e12, up, amount); B = o['B']
    print(f"amount={amount} eroded={B['eroded']!r} land={B['land']!r} lakes={B['lakes']!r} delta={B['delta']!r} ocean={B['ocean']!r} resid={budget_residual(B):.1e}")
    print('  lake dh', [o['dh'][3*NXB+4], o['dh'][3*NXB+5]], 'channel dh', [o['dh'][3*NXB+i] for i in range(0, 4)], 'flux channel', [o['flux'][3*NXB+i] for i in range(0, 4)])
    print('  units', o['units'], 'lake', [c for c in range(n) if o['lake'][c] >= 0], 'delta', [c for c in range(n) if o['delta'][c]], 'fp', [c for c in range(n) if o['fp'][c]], 'any land dh', [(c, round(o['dh'][c], 4)) for c in range(n) if o['dh'][c] > 0 and not R['ocean'][c] and o['lake'][c] < 0])
