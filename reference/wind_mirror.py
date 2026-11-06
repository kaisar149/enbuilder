"""NumPy mirror of Part 3a: PressureModel + DiagnosticWind (same formulas as src/pressure.rs, src/wind.rs)."""
import numpy as np, math, heapq
from energy_mirror import mesh

R_GAS=287.05; P0=101325.0
PP=dict(rotation_period_s=86164.0, coriolis_scale=1.0, coriolis_const=0.0, thermal_coeff=0.04, band_coeff=0.05,
        hadley_scale=2.5, contrast_k=3.36, upper_ratio=0.7, sea_level=0.0, atm_density=1.2)
WP=dict(drag_ocean=1.5e-5, drag_land=3.0e-5, drag_rough_ref_m=500., drag_upper=8.0e-6, upper_density_ratio=0.5,
        stability_n=0.01, blocking_gain=1.0, max_wind=80., sea_level=0.0)

def frames(M, axis):
    up=M['up']; n=up.shape[0]
    north=axis[None,:]-up*(up@axis)[:,None]
    ln=np.linalg.norm(north,axis=1)
    for i in np.where(ln<1e-9)[0]:
        alt=np.array([1.,0,0]) if abs(up[i,0])<.9 else np.array([0,1.,0])
        north[i]=alt-up[i]*(alt@up[i])
    north/=np.linalg.norm(north,axis=1)[:,None]
    east=np.cross(north,up)
    return north,east

def edge_vec(M,i,j,dist):
    v=M['cen'][j]-M['cen'][i]; L=np.linalg.norm(v)
    if L>1.5*dist: v=-v/L*dist   # wrap-around edge
    return v

def grad_op(M, north, east):
    """per cell: list of (j, world tangent coefficient vector c) with grad = sum c*(phi_j-phi_i)"""
    ops=[]
    for i in range(M['n']):
        D=[];W=[];J=[]
        for (j,l,d) in M['nb'][i]:
            v=edge_vec(M,i,j,d)
            v=v-M['up'][i]*(v@M['up'][i])
            ln=np.linalg.norm(v)
            if ln<1e-12: continue
            v=v/ln*d
            D.append([v@east[i],v@north[i]]); W.append(1.0/(d*d)); J.append(j)
        D=np.array(D);W=np.array(W)
        Mx=(D*W[:,None]).T@D
        Mx+=np.eye(2)*1e-6*np.trace(Mx)
        Mi=np.linalg.inv(Mx)
        cs=(Mi@(D*W[:,None]).T).T   # (k,2)
        ops.append([(J[k], cs[k,0]*east[i]+cs[k,1]*north[i]) for k in range(len(J))])
    return ops

def grad(ops,phi):
    g=np.zeros((len(ops),3))
    for i,o in enumerate(ops):
        for j,c in o: g[i]+=c*(phi[j]-phi[i])
    return g

def pressure(M,Tsl,T,z,axis,p=PP):
    n=M['n']; A=M['area']; up=M['up']
    p0=P0*p['atm_density']/1.2
    wsum=A.sum()
    Tbar=(Tsl*A).sum()/wsum
    sd=math.sqrt(((Tsl-Tbar)**2*A).sum()/wsum)
    dT=p['contrast_k']*sd
    pth=-p['thermal_coeff']*p0*(Tsl-Tbar)/Tbar
    Om=0.0 if p['rotation_period_s']<=0 else 2*math.pi/p['rotation_period_s']
    f=p['coriolis_scale']*(p['coriolis_const'] if p['coriolis_const']!=0 else 2*Om*(up@axis))
    band=np.full(n,np.nan); pdyn=np.zeros(n); LH=float('nan')
    fmax=np.abs(f).max()
    if dT>0 and fmax>0:
        Oeff=fmax/2
        # seeds at f sign changes
        d=np.full(n,np.inf)
        for i in range(n):
            if f[i]==0: d[i]=0.0
            for (j,l,dd) in M['nb'][i]:
                if f[i]*f[j]<0:
                    d[i]=min(d[i],dd*abs(f[i])/(abs(f[i])+abs(f[j])))
        h=[(d[i],i) for i in range(n) if np.isfinite(d[i])]; heapq.heapify(h)
        while h:
            di,i=heapq.heappop(h)
            if di>d[i]: continue
            for (j,l,dd) in M['nb'][i]:
                nd=di+dd
                if nd<d[j]: d[j]=nd; heapq.heappush(h,(nd,j))
        if np.isfinite(d).any():
            c2=(5/3)*p['hadley_scale']*R_GAS*dT
            LH=math.sqrt(c2)/Oeff
            amp=p['band_coeff']*p0*dT/Tbar
            ok=np.isfinite(d)
            band[ok]=d[ok]/LH
            pdyn[ok]=-amp*np.cos(math.pi*band[ok])
    anom=pth+pdyn
    anom-= (anom*A).sum()/wsum
    psl=p0+anom
    zs=np.maximum(z,p['sea_level'])
    g=9.81
    Tcol=0.5*(Tsl+T)
    ps=psl*np.exp(-g*zs/(R_GAS*Tcol)) if True else psl
    rho=ps/(R_GAS*T)
    pup=0.5*p0-p['upper_ratio']*anom
    return dict(psl=psl,ps=ps,pup=pup,rho=rho,f=f,band=band,LH=LH,dT=dT,p0=p0)

def wind(M,ops,north,east,F,z,p=WP):
    n=M['n']; up=M['up']; A=M['area']
    zc=np.maximum(z,p['sea_level'])
    rough=np.zeros(n)
    for i in range(n):
        nbs=M['nb'][i]
        rough[i]=np.mean([abs(zc[j]-zc[i]) for (j,_,_) in nbs])
    base=np.where(z<p['sea_level'],p['drag_ocean'],p['drag_land'])
    r=base*(1+rough/p['drag_rough_ref_m'])
    f=F['f']
    def solve(pr,rho,r):
        a=-grad(ops,pr)/rho[:,None]
        kxa=np.cross(up,a)
        den=(r*r+f*f)[:,None]
        return (r[:,None]*a-f[:,None]*kxa)/den
    u=solve(F['psl'],F['rho'],r)
    gz=grad(ops,zc)
    s=np.linalg.norm(gz,axis=1)
    if p['blocking_gain']>0:
        U=np.linalg.norm(u,axis=1)
        rise=s*np.sqrt(A)
        for i in range(n):
            if s[i]>1e-9:
                gh=gz[i]/s[i]; w=u[i]@gh
                if w>0:
                    b=min(1.0,p['blocking_gain']*rise[i]/(rise[i]+U[i]/p['stability_n']+1e-12))
                    u[i]-=b*w*gh
    def cap(u):
        S=np.linalg.norm(u,axis=1); k=np.ones(n)
        m=S>1e-12; k[m]=np.tanh(S[m]/p['max_wind'])*p['max_wind']/S[m]
        return u*k[:,None]
    u=cap(u)
    rup=np.full(n,p['drag_upper'])
    uu=cap(solve(F['pup'],F['rho']*p['upper_density_ratio'],rup))
    return u,uu
