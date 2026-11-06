from geo_mirror import *
M=mesh('sphere',4,6.371e6); up=M['up']; n=M['n']
plate=(up[:,0]<0).astype(int); pt=(plate,[np.array([-1.,0,0]),np.array([1.,0,0])],np.array([1,1]))
r=tectonics(M,painted=pt); te=r['elev']; st=r['strength']
print("tect max",te.max().round(0))
for g in (3.0,9.81,25.0):
    e,sl=terrain(M,te,st,g=g)
    print(f"g={g:5.2f} max elev {e.max():7.0f} min {e.min():7.0f}  max slope {sl.max():.4f}  mean slope {sl.mean():.5f} relief std {e.std():.0f}")
# slope-cap effect
e0,s0=terrain(M,te,st,iters=0,cap=False,noise=False)
e1,s1=terrain(M,te,st,iters=8,slope_ref=.005,cap=False,noise=False)
print("max slope no erosion",s0.max().round(4),"with slope_ref .005:",s1.max().round(4))
V0=(e0*M['area']).sum();V1=(e1*M['area']).sum();print("volume change rel",(V1-V0)/np.abs(e0*M['area']).sum())
# noise determinism & magnitude
e2,_=terrain(M,te,st,seed=3,cap=False,iters=0); e3,_=terrain(M,te,st,seed=3,cap=False,iters=0); e4,_=terrain(M,te,st,seed=4,cap=False,iters=0)
print("noise det",np.array_equal(e2,e3),"differs by seed",not np.array_equal(e2,e4),"noise std",(e2-te).std().round(0),"max",abs(e2-te).max().round(0))
# random world heights
r=tectonics(M,seed=5); e,sl=terrain(M,r['elev'],r['strength']); print("random world elev",e.min().round(0),e.max().round(0),"slope max",sl.max().round(4))
