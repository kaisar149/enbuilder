import math
from mirror_4c2 import *
def multi(name, W, z, oc, steps, Kc, years, ms, df=0.5, void=False):
    n = W['n']; st = [1.0]*n; g = [G_EARTH]*n; z = list(z); cum_exp = 0.0; cum_er = 0.0
    z0 = list(z); R0 = route(W, z, oc, void); ndep0 = sum(d > 0 for d in R0['dep'])
    land = [c for c in range(n) if not oc[c]]
    peak0 = max(z[c] for c in land); mean0 = sum(z[c] for c in land)/len(land)
    vol0 = sum(W['area'][c]*z[c] for c in range(n))
    deps = []
    for s in range(steps):
        R = route(W, z, oc, void); deps.append(sum(d > 0 for d in R['dep']))
        es = full_erosion(W, z, R, st, g, dict(EROS, years=years))
        p = dict(DEF, Kc=Kc, thermal_iters=0 if ms is None else 5000, max_slope=ms or 0.05, years=years)
        o = sediment_step(W, z, es, R, st, g, p); B = o['B']
        assert budget_residual(B) < 1e-12
        cum_exp += B['ocean']+B['void']+B['sink']; cum_er += B['eroded']
        z = list(o['z3'])
    R = route(W, z, oc, void); deps.append(sum(d > 0 for d in R['dep']))
    vol1 = sum(W['area'][c]*z[c] for c in range(n))
    peak1 = max(z[c] for c in land); mean1 = sum(z[c] for c in land)/len(land)
    print(f"{name:9s} Kc={Kc:.0e} ms={ms} dep={deps} peak {peak0:.0f}->{peak1:.0f} mean {mean0:.0f}->{mean1:.0f} dV/export={(vol1-vol0)/cum_exp:+.3e} (eroded {cum_er:.2e}) ")
W = sphere_world(3); zz = m41.terrain(W, 3); oc = m41.ocean_cells(W, zz)
for Kc in (1e9, 1e10, 1e11):
  for ms in (None, 2e-4):
    multi('sphere3', W, list(zz), oc, 10, Kc, 1e5, ms)
W = sphere_world(4); zz = m41.terrain(W, 11); oc = m41.ocean_cells(W, zz)
multi('sphere4', W, list(zz), oc, 8, 1e10, 1e5, 2e-4)
