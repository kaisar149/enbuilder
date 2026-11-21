import math
from mirror_4c2 import *
NX, NY = 16, 5
def build():
    W = plane_world(NX, NY); n = W['n']; z = [0.0]*n
    for j in range(NY):
        for i in range(NX):
            if i <= 1: h = -20.0
            elif i <= 9: h = 10.0 + (i-1)*2.0
            else: h = 26.0 + (i-9)*300.0
            z[j*NX+i] = h + 50.0*abs(j-2) if i >= 2 else h
    return W, z
W, z = build(); n = W['n']
oc = [z[c] < 0 for c in range(n)]
R = route(W, z, oc)
st = [1.0]*n; g = [G_EARTH]*n
print('dep cells', sum(d > 0 for d in R['dep']), 'ocean', sum(oc))
es = full_erosion(W, z, R, st, g, dict(EROS, years=1e5))
print('erosion steps row2:', [round(es[2*NX+i], 3) for i in range(NX)])
print('drain area row2/1e8:', [round(R['area'][2*NX+i]/1e8) for i in range(NX)])
for Kc in (5e7, 2e7, 1e7, 1e6):
  for df in (0.5,):
    p = dict(DEF, Kc=Kc, deposit_fraction=df, thermal_iters=400)
    o = sediment_step(W, z, es, R, st, g, p); B = o['B']
    print(f"Kc={Kc:.0e} df={df} resid={budget_residual(B):.1e} eroded={B['eroded']:.4e} land={B['land']:.4e} delta={B['delta']:.4e} lakes={B['lakes']:.4e} ocean={B['ocean']:.4e}")
    print('  dh row2:', [round(o['dh'][2*NX+i], 4) for i in range(NX)])
    print('  S   row2:', [round(o['slope'][2*NX+i], 4) for i in range(NX)])
    print('  fp', [i for i in range(n) if o['fp'][i]], 'delta', [i for i in range(n) if o['delta'][i]])
