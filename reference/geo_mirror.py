import numpy as np, math, heapq
import mirror as m
from energy_mirror import mesh
def proj(v,up): return v-up*(v@up)
def unit(v):
    l=np.linalg.norm(v); return v/l if l>0 else v
TP=dict(n_plates=12,cont_frac=.4,speed=1.0,width=0.0,hotspots=3,base_c=400.,base_o=-3800.,max_up=8000.,max_trench=6000.,rift=1500.,ridge=2000.,hot_up=1200.,trans_thr=.15)
def propagate(M,seed,plate,L):
    n=M['n'];val=np.zeros(n);h=[]
    for i in np.nonzero(seed>0)[0]: val[i]=seed[i]; heapq.heappush(h,(-seed[i],i))
    done=np.zeros(n,bool)
    while h:
        v,i=heapq.heappop(h);v=-v
        if done[i]: continue
        done[i]=True
        for j,l,d in M['nb'][i]:
            if plate[j]!=plate[i]: continue
            nv=v*math.exp(-d/L)
            if nv>1e-3 and nv>val[j]: val[j]=nv; heapq.heappush(h,(-nv,j))
    return val
def tectonics(M,seed=1,p=TP,painted=None):
    rng=np.random.default_rng(seed);n=M['n']
    if painted is None:
        seeds=rng.choice(n,p['n_plates'],replace=False)
        gw=rng.uniform(.5,1.5,p['n_plates']); plate=-np.ones(n,int); h=[]
        for k,s in enumerate(seeds): heapq.heappush(h,(0.0,s,k))
        while h:
            d,i,k=heapq.heappop(h)
            if plate[i]>=0: continue
            plate[i]=k
            for j,l,dd in M['nb'][i]:
                if plate[j]<0: heapq.heappush(h,(d+dd/gw[k],j,k))
        cont=rng.random(p['n_plates'])<p['cont_frac']
        V=[]
        for s in seeds:
            t=proj(rng.normal(size=3),M['up'][s]);V.append(unit(t)*p['speed']*rng.uniform(.3,1))
    else: plate,V,cont=painted
    np_=len(V)
    vel=np.array([proj(V[plate[i]],M['up'][i]) for i in range(n)])
    meanD=np.mean([d for i in range(n) for _,_,d in M['nb'][i]])
    L=p['width'] or 3*meanD
    up_s=np.zeros(n);tr_s=np.zeros(n);vo_s=np.zeros(n);rf_s=np.zeros(n);rd_s=np.zeros(n);btype=np.zeros(n,int);best=np.zeros(n)
    for i in range(n):
        for j,l,d in M['nb'][i]:
            if plate[i]==plate[j]: continue
            nrm=unit(proj(M['cen'][j]-M['cen'][i],M['up'][i]))
            c=(vel[i]-vel[j])@nrm; s=min(1,abs(c)/(1.5*p['speed']))
            ci,cj=cont[plate[i]],cont[plate[j]]
            if abs(c)<p['trans_thr']*p['speed']: code=4
            elif c>0:
                if ci and cj: code=1;up_s[i]=max(up_s[i],s)
                elif ci and not cj: code=2;up_s[i]=max(up_s[i],.5*s);vo_s[i]=max(vo_s[i],s)
                elif (not ci) and cj: code=2;tr_s[i]=max(tr_s[i],s)
                else:
                    if plate[i]<plate[j]: code=2;up_s[i]=max(up_s[i],.6*s);vo_s[i]=max(vo_s[i],s)
                    else: code=2;tr_s[i]=max(tr_s[i],s)
            else:
                code=3
                if ci: rf_s[i]=max(rf_s[i],s);vo_s[i]=max(vo_s[i],.3*s)
                else: rd_s[i]=max(rd_s[i],s);vo_s[i]=max(vo_s[i],.2*s)
            if abs(c)>=best[i]: best[i]=abs(c);btype[i]=code
    up_f=propagate(M,up_s,plate,L);tr_f=propagate(M,tr_s,plate,.5*L);rf_f=propagate(M,rf_s,plate,L);rd_f=propagate(M,rd_s,plate,L);vo_f=propagate(M,vo_s,plate,.5*L)
    hs=np.zeros(n)
    if painted is None and p['hotspots']>0: hs[rng.choice(n,p['hotspots'],replace=False)]=1
    hs_f=propagate(M,hs,plate,.7*L)
    c_i=cont[plate]
    base=np.where(c_i,p['base_c'],p['base_o'])
    elev=base+up_f*p['max_up']-tr_f*p['max_trench']-rf_f*p['rift']+rd_f*p['ridge']+hs_f*p['hot_up']
    strength=np.where(c_i,1.0,1.25)*(1+.3*up_f)*(1-.3*rf_f); strength=np.clip(strength,.3,2.)
    volc=np.minimum(1,np.maximum(vo_f,hs_f))
    return dict(plate=plate,cont=c_i,btype=btype,elev=elev,strength=strength,volc=volc)
def soft_cap(e,h,k=.6):
    e=np.asarray(e,float);out=e.copy();m_=e>k*h
    out[m_]=k*h+(1-k)*h*np.tanh((e[m_]-k*h)/((1-k)*h));return out
def vnoise(P,seed=0):
    def lat(ix,iy,iz):
        h=(ix*374761393+iy*668265263+iz*2147483647+seed*1274126177)&0xFFFFFFFF
        h=((h^(h>>13))*1274126177)&0xFFFFFFFF; h^=h>>16; return h/2**31-1
    f=np.floor(P);t=P-f;t=t*t*(3-2*t);ix,iy,iz=[int(v) for v in f]
    out=0
    for dx in (0,1):
        for dy in (0,1):
            for dz in (0,1):
                w=(t[0] if dx else 1-t[0])*(t[1] if dy else 1-t[1])*(t[2] if dz else 1-t[2]); out+=w*lat(ix+dx,iy+dy,iz+dz)
    return out
def terrain(M,tect_elev,strength,g=9.81,noise_amp=600.,slope_ref=.05,iters=8,peak_ref=9000.,depth_ref=11000.,seed=0,a=.15,noise=True,cap=True):
    n=M['n'];gref=9.81;gm=np.full(n,g) if np.isscalar(g) else g
    e=tect_elev.copy()
    if noise and noise_amp>0:
        ext=np.ptp(M['cen'],axis=0).max();wl=ext/5
        for i in range(n):
            s=0;amp=1;tot=0;f=1
            for o in range(5): s+=amp*vnoise(M['cen'][i]/wl*f,seed+o);tot+=amp;amp*=.5;f*=2
            e[i]+=noise_amp*(s/tot)*gref/gm[i]
    for _ in range(iters):
        d=np.zeros(n)
        for i in range(n):
            for j,l,dd in M['nb'][i]:
                lim=slope_ref*(gref/(.5*(gm[i]+gm[j])))*.5*(strength[i]+strength[j])*dd
                if e[i]-e[j]>lim: am=a*(e[i]-e[j]-lim);d[i]-=am;d[j]+=am*M['area'][i]/M['area'][j]
        e+=d
    if cap:
        hp=strength*peak_ref*gref/gm;hn=depth_ref*gref/gm
        e=np.where(e>=0,soft_cap(np.maximum(e,0),hp) if np.ndim(hp)==0 else np.array([soft_cap([max(e[i],0)],hp[i])[0] for i in range(n)]),-soft_cap(np.maximum(-e,0),hn) if np.ndim(hn)==0 else -np.array([soft_cap([max(-e[i],0)],hn[i])[0] for i in range(n)]))
    sl=np.zeros(n)
    for i in range(n):
        for j,l,dd in M['nb'][i]: sl[i]=max(sl[i],abs(e[i]-e[j])/dd)
    return e,sl
