import sys, math, random
from mirror_4c2 import *
def check(name, W, z, oc, void=False, Kc=1e9, years=1e6, df=0.5, ms=None):
    n = W['n']; st = [1.0]*n; g = [G_EARTH]*n; z = list(z)
    R = route(W, z, oc, void)
    es = full_erosion(W, z, R, st, g, dict(EROS, years=years))
    p = dict(DEF, Kc=Kc, deposit_fraction=df, thermal_iters=0 if ms is None else 3000, max_slope=ms or 0.05)
    o = sediment_step(W, z, es, R, st, g, p); B = o['B']
    exp = B['ocean'] + B['void'] + B['sink']
    topo = sum(W['area'][c]*(o['z3'][c]-z[c]) for c in range(n))
    z2 = o['z2']
    # ordering preserved before thermal
    worst = 0.0
    for c in range(n):
        r = R['fd'][c]
        if r < 0 or R['ocean'][c]: continue
        ref = R['filled'][c] if o['lake'][c] >= 0 else z2[c]
        # a receiver must not be above the donor's effective level
        worst = max(worst, z2[r] - (R['filled'][c] if o['lake'][c] >= 0 else z2[c]) if o['lake'][c] < 0 or r not in o['units'][o['lake'][c]] else 0.0)
    assert min(o['dh']) >= 0
    R2 = route(W, o['z3'], oc, void); n0 = sum(d > 0 for d in R['dep']); n1 = sum(d > 0 for d in R2['dep'])
    print(f"{name:10s} n={n:5d} resid={budget_residual(B):.0e} topo+exp={(topo+exp)/max(B['eroded'],1):+.0e} order_viol={worst:.1e} dep {n0}->{n1} unres={o['n_unresolved']} land={B['land']/B['eroded']:.2f} delta={B['delta']/B['eroded']:.2f} lakes={B['lakes']/B['eroded']:.2f} exp={exp/B['eroded']:.2f} sink={B['sink']/B['eroded']:.2f} void={B['void']/B['eroded']:.2f} it={o['iters']} exc={o['excess']:.0e} tvol={o['tvol']/B['eroded']:+.0e}")
    assert budget_residual(B) < 1e-12 and abs(topo+exp) < 1e-12*B['eroded']
    assert worst <= 1e-9, worst
    return o
for Kc in (1e9, 1e12):
  for ms in (None, 1e-4):
    print('Kc', Kc, 'ms', ms)
    W = sphere_world(3); zz = m41.terrain(W, 3); check('sphere3', W, list(zz), m41.ocean_cells(W, zz), Kc=Kc, ms=ms)
    W = sphere_world(4); zz = m41.terrain(W, 11); check('sphere4', W, list(zz), m41.ocean_cells(W, zz), Kc=Kc, ms=ms)
    W = cube_world(14); zz = m41.terrain(W, 12); check('cube14', W, list(zz), m41.ocean_cells(W, zz), Kc=Kc, ms=ms)
    W = plane_world(40, 24); zz = m41.terrain(W, 13); check('wallplane', W, list(zz), m41.ocean_cells(W, zz), Kc=Kc, ms=ms)
    W = plane_world(40, 24, wrap=True); zz = m41.terrain(W, 14); check('wrapplane', W, list(zz), m41.ocean_cells(W, zz), Kc=Kc, ms=ms)
    W = plane_world(40, 24); zz = m41.terrain(W, 15) + 400; check('voidplane', W, list(zz), [False]*W['n'], void=True, Kc=Kc, ms=ms)
    W = plane_world(24, 16); zz = m41.terrain(W, 16) + 400; check('closed', W, list(zz), [False]*W['n'], Kc=Kc, ms=ms)
