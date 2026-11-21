import sys, math
from mirror_4c2 import *
def world(level, seed):
    W = sphere_world(level); zz = world_terrain(W, seed); oc = m41.ocean_cells(W, zz); return W, list(zz), oc
for level, seed in ((3, 3), (4, 5)):
    W, z, oc = world(level, seed); n = W['n']; st = [1.0] * n; g = [G_EARTH] * n
    R = route(W, z, oc); ndep0 = sum(d > 0 for d in R['dep'])
    es = full_erosion(W, z, R, st, g, dict(EROS, years=1e6))
    for Kc in (1e9, 1e11, 1e12):
        for ms in (None, 2e-4, 1e-4):
            p = dict(DEF, Kc=Kc, thermal_iters=0 if ms is None else 5000, max_slope=ms or 0.05, thermal_tol_m=1e-6)
            o = sediment_step(W, z, es, R, st, g, p); B = o['B']
            R2 = route(W, o['z3'], oc); ndep1 = sum(d > 0 for d in R2['dep'])
            # slope check
            worst = 0.0
            if ms:
                for i in range(n):
                    for k, j in enumerate(W['adj'][i]):
                        lim = p['talus_scale'] * ms * W['ed'][i][k]
                        worst = max(worst, abs(o['z3'][i] - o['z3'][j]) - lim)
            print(f"L{level} Kc={Kc:.0e} ms={ms} land={B['land']/B['eroded']:.3f} delta={B['delta']/B['eroded']:.3f} lakes={B['lakes']/B['eroded']:.3f} ocean={B['ocean']/B['eroded']:.3f} resid={budget_residual(B):.0e} it={o['iters']} exc={o['excess']:.1e} worst={worst:.1e} dep {ndep0}->{ndep1} fp={sum(o['fp'])} dm={sum(o['delta'])} tvol={o['tvol']:.1e}")
