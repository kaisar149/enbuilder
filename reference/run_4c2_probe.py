import sys, math, random
import numpy as np
from mirror_4c2 import *
def world(level, seed):
    W = sphere_world(level); z = world_terrain(W, seed); oc = m41.ocean_cells(W, z); return W, z, oc
for level, seed in ((3, 3), (4, 5)):
    W, z, oc = world(level, seed); n = W['n']; st = [1.0] * n; g = [G_EARTH] * n
    z = list(z); R = route(W, z, oc)
    es = full_erosion(W, z, R, st, g, dict(EROS, years=1e6))
    for Kc in (2e5, 2e3, 20.0, 0.0):
        for tm in (0, 400):
            p = dict(DEF, Kc=Kc, thermal_iters=tm, max_slope=0.002)
            o = sediment_step(W, z, es, R, st, g, p)
            B = o['B']; res = budget_residual(B)
            topo = sum(W['area'][c] * (o['z3'][c] - z[c]) for c in range(n))
            exp = B['ocean'] + B['void'] + B['sink']
            print(f"L{level} Kc={Kc:8.1f} th={tm:3d} eroded={B['eroded']:.3e} land={B['land']/B['eroded']:.3f} delta={B['delta']/B['eroded']:.3f} lakes={B['lakes']/B['eroded']:.3f} ocean={B['ocean']/B['eroded']:.3f} sink={B['sink']:.1e} resid={res:.1e} topo+export={(topo+exp)/B['eroded']:+.1e} tvol={o['tvol']:.1e} thermal_it={o['iters']} exc={o['excess']:.1e} unres={o['n_unresolved']} dmask={sum(o['delta'])} fp={sum(o['fp'])}")
