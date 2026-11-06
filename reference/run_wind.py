import numpy as np, math
from energy_mirror import mesh
from wind_mirror import *
R=6.371e6
def setup(level):
    M=mesh('sphere',level,R)
    axis=np.array([0,0,1.])
    north,east=frames(M,axis)
    ops=grad_op(M,north,east)
    return M,axis,north,east,ops
def temps(M,dT=40.):
    mu=M['up'][:,2]
    Tsl=300-dT*mu**2
    return Tsl,Tsl.copy()
def zonal(M,north,east,u,mu_bins):
    ue=(u*east).sum(1); out=[]
    for a,b in mu_bins:
        m=(M['up'][:,2]>=a)&(M['up'][:,2]<b); out.append(ue[m].mean())
    return np.array(out)
def count_extrema(M,P):
    mu=M['up'][:,2]; edges=np.linspace(0,1,41)
    zp=[P[(abs(mu)>=a)&(abs(mu)<b)].mean() for a,b in zip(edges[:-1],edges[1:])]
    # use latitude bins instead of mu for equal spacing
    lat=np.degrees(np.arcsin(abs(mu))); ed=np.linspace(0,90,31)
    zp=np.array([P[(lat>=a)&(lat<b)].mean() for a,b in zip(ed[:-1],ed[1:])])
    d=np.diff(zp); return int(((d[1:]*d[:-1])<0).sum()), zp
if __name__=="__main__":
    import sys
    level=int(sys.argv[1]) if len(sys.argv)>1 else 4
    M,axis,north,east,ops=setup(level)
    z=np.full(M['n'],-3000.)
    Tsl,T=temps(M)
    for name,per in [('0.25x',86164*4),('1x',86164.),('3x',86164/3),('tidal(none)',0.)]:
        pp=dict(PP); pp['rotation_period_s']=per
        F=pressure(M,Tsl,T,z,axis,pp)
        u,uu=wind(M,ops,north,east,F,z)
        ne,zp=count_extrema(M,F['psl'])
        sp=np.linalg.norm(u,axis=1)
        lat=np.degrees(np.arcsin(M['up'][:,2]))
        bins=[(0,30),(30,60),(60,90)]
        ue=(u*east).sum(1)
        zon=[ue[(abs(lat)>=a)&(abs(lat)<b)].mean() for a,b in bins]
        print(name,'L_H km',F['LH']/1e3 if F['LH']==F['LH'] else None,'extrema',ne,'mean|u|',sp.mean().round(2),'max',sp.max().round(1),'zonal u [0-30,30-60,60-90]',np.round(zon,2),'p range hPa',((F['psl'].max()-F['psl'].min())/100).round(1))
