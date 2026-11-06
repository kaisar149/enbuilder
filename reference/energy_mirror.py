import numpy as np, math
import mirror as m
SIG=5.670374419e-8
def mesh(kind,*a,**k):
    area,nb,_=getattr(m,kind)(*a,**k)
    return dict(n=len(nb),area=area,cen=m.CEN['c'].copy(),up=m.CEN['u'].copy(),nb=nb)
def rot(v,ax,th): return v*math.cos(th)+np.cross(ax,v)*math.sin(th)+ax*(ax@v)*(1-math.cos(th))
def sun_dirs(d0,omega,tilt,orbit,day,season,K):
    d0=d0/np.linalg.norm(d0)
    if orbit:
        a=d0-omega*(d0@omega); a/=np.linalg.norm(a)
        N=omega*math.cos(tilt)+np.cross(a,omega)*math.sin(tilt)
        ph=2*math.pi*season; s=a*math.cos(ph)+np.cross(N,a)*math.sin(ph)
    else: s=d0
    if day>0: return [rot(s,omega,-2*math.pi*(k+.5)/K) for k in range(K)]
    return [s]
def insolation_dir(M,I,d0,omega,tilt,orbit,day,season,K=24):
    ds=sun_dirs(d0,omega,tilt,orbit,day,season,K)
    return I*np.mean([np.maximum(0,M['up']@s) for s in ds],axis=0)
def insolation_point(M,I,pos,ref):
    d=pos-M['cen']; r=np.linalg.norm(d,axis=1); c=np.maximum(0,(M['up']*d).sum(1)/r)
    return I*(ref/r)**2*c
def albedo(T,elev,p):
    f=np.clip((p['ice_hi']-T)/(p['ice_hi']-p['ice_lo']),0,1)
    surf=np.where(elev<p['sea'],p['a_water'],p['a_land'])
    a0=f*p['a_ice']+(1-f)*surf
    return a0+(1-a0)*p['cloud'],f
def eb_solve(M,Q,alb,elev,Tsl0,p):
    n=M['n'];A=M['area'];Lref2=A.sum()/(4*math.pi);Kt=p['D']*Lref2
    eps=1-p['gh']
    # graph weights
    rows=[[(j,l/d) for j,l,d in M['nb'][i]] for i in range(n)]
    Wi=np.array([sum(w for _,w in r) for r in rows])
    def lap(x): return np.array([sum(w*(x[j]-x[i]) for j,w in rows[i]) for i in range(n)])
    T=Tsl0.copy()
    for it in range(p['newton']):
        R=A*((1-alb)*Q-eps*SIG*T**4)+Kt*lap(T)
        diag=A*4*eps*SIG*T**3+Kt*Wi
        def mv(x): return diag*x-Kt*(-lap(x)+Wi*x)*(-1)*(-1) if False else diag*x-Kt*np.array([sum(w*x[j] for j,w in rows[i]) for i in range(n)])
        x=np.zeros(n);r=R.copy();z=r/diag;pv=z.copy();rz=r@z;bn=np.linalg.norm(R)+1e-300
        for k in range(p['cg_max']):
            Ap=mv(pv);al=rz/(pv@Ap);x+=al*pv;r-=al*Ap
            if np.linalg.norm(r)<=p['cg_tol']*bn: break
            z=r/diag;rz2=r@z;pv=z+rz2/rz*pv;rz=rz2
        x=np.clip(x,-40,40);T=np.clip(T+x,50,600)
        if np.abs(x).max()<1e-4: break
    return T
P=dict(ice_lo=263.15,ice_hi=273.15,a_land=.28,a_water=.08,a_ice=.62,cloud=.15,sea=0.0,D=.55,gh=.39,newton=8,cg_tol=1e-8,cg_max=3000,lapse=.65)
def loop(M,Q,elev,p=P,om=.5,tol=1e-4,maxit=50,T0=288.0,g=9.81,cp=1005.):
    lapse=p['lapse']*g/cp; h=np.maximum(elev,0)
    Tsl=np.full(M['n'],T0); Ts=Tsl-lapse*h
    for it in range(1,maxit+1):
        alb,f=albedo(Ts,elev,p)
        Tn=eb_solve(M,Q,alb,elev,Tsl,p); Tsn=Tn-lapse*h
        Tr=Ts+om*(Tsn-Ts); Tslr=Tsl+om*(Tn-Tsl)
        res=max(np.abs(Tslr-Tsl).max()/Tslr.max(),np.abs(Tr-Ts).max()/Tr.max())
        Tsl,Ts=Tslr,Tr
        if res<=tol: return it,Ts,f
    return maxit,Ts,f
