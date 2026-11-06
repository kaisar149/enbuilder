from geo_mirror import *
M=mesh('sphere',4,6.371e6); up=M['up']; n=M['n']
# painted hemispheres, velocities along x
def paint(vA,vB,cA,cB):
    plate=(up[:,0]<0).astype(int)   # plate0: x>=0, plate1: x<0
    return plate,[np.array(vA,float),np.array(vB,float)],np.array([cA,cB])
bd=abs(up[:,0])<.1; inter=abs(up[:,0])>.8
def rep(name,pt):
    r=tectonics(M,painted=pt)
    e=r['elev'];print(f"{name:12s} boundary mean {e[bd].mean():8.0f}  interior(x>0) {e[inter&(up[:,0]>0)].mean():8.0f} interior(x<0) {e[inter&(up[:,0]<0)].mean():8.0f}  bdary x>0 side {e[(up[:,0]>0)&(up[:,0]<.1)].mean():8.0f} x<0 side {e[(up[:,0]<0)&(up[:,0]>-.1)].mean():8.0f} volc max {r['volc'].max():.2f} types {sorted(set(r['btype'][bd]))}")
    return r
rep("collision",paint([-1,0,0],[1,0,0],1,1))
rep("rift cont",paint([1,0,0],[-1,0,0],1,1))
rep("rift ocean",paint([1,0,0],[-1,0,0],0,0))
rep("subduction",paint([-1,0,0],[1,0,0],1,0))
rep("transform",paint([0,0,1],[0,0,1],1,1))
# random worlds: several seeds
for sd in range(1,7):
    r=tectonics(M,seed=sd); cnt={k:(r['btype']==k).sum() for k in range(5)}
    print("seed",sd,"plates",len(set(r['plate'])),"elev range",r['elev'].min().round(0),r['elev'].max().round(0),"land frac",(r['elev']>0).mean().round(2),cnt,"volc",r['volc'].min().round(2),r['volc'].max().round(2),"str",r['strength'].min().round(2),r['strength'].max().round(2))
