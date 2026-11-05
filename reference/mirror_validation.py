import math, itertools
import numpy as np
def norm(v): return v/np.linalg.norm(v)
def arc(a,b): return math.atan2(np.linalg.norm(np.cross(a,b)), a@b)
def sph_tri(a,b,c):
    t=abs(a@np.cross(b,c)); d=1+a@b+b@c+c@a
    return 2*math.atan2(t,d)
def ico():
    t=(1+5**.5)/2
    V=[(-1,t,0),(1,t,0),(-1,-t,0),(1,-t,0),(0,-1,t),(0,1,t),(0,-1,-t),(0,1,-t),(t,0,-1),(t,0,1),(-t,0,-1),(-t,0,1)]
    F=[(0,11,5),(0,5,1),(0,1,7),(0,7,10),(0,10,11),(1,5,9),(5,11,4),(11,10,2),(10,7,6),(7,1,8),(3,9,4),(3,4,2),(3,2,6),(3,6,8),(3,8,9),(4,9,5),(2,4,11),(6,2,10),(8,6,7),(9,8,1)]
    return [norm(np.array(v,float)) for v in V],[tuple(f) for f in F]
def sphere(level,R=1.0):
    V,F=ico()
    for _ in range(level):
        cache={};NF=[]
        def mid(a,b):
            k=(min(a,b),max(a,b))
            if k not in cache:
                V.append(norm(V[a]+V[b])); cache[k]=len(V)-1
            return cache[k]
        for a,b,c in F:
            ab,bc,ca=mid(a,b),mid(b,c),mid(c,a)
            NF+= [(a,ab,ca),(b,bc,ab),(c,ca,bc),(ab,bc,ca)]
        F=NF
    n=len(V); inc=[[] for _ in range(n)]
    for ti,f in enumerate(F):
        for v in f: inc[v].append(ti)
    cen=[norm(V[a]+V[b]+V[c]) for a,b,c in F]
    area=np.zeros(n); nb=[[] for _ in range(n)]
    for v in range(n):
        p=V[v]; e1=np.cross(p,[1,0,0]) 
        if np.linalg.norm(e1)<.5: e1=np.cross(p,[0,1,0])
        e1=norm(e1); e2=np.cross(p,e1)
        ts=sorted(inc[v],key=lambda t: math.atan2(cen[t]@e2,cen[t]@e1))
        m=len(ts)
        for k in range(m):
            t0,t1=ts[k],ts[(k+1)%m]
            sh=(set(F[t0])&set(F[t1]))-{v}; assert len(sh)==1
            w=sh.pop()
            area[v]+=sph_tri(p,cen[t0],cen[t1])*R*R
            nb[v].append((w,arc(cen[t0],cen[t1])*R,arc(p,V[w])*R))
    return area,nb,4*math.pi*R*R
FACES=[((1,0,0),(0,1,0),(0,0,1)),((-1,0,0),(0,1,0),(0,0,1)),((0,1,0),(1,0,0),(0,0,1)),((0,-1,0),(1,0,0),(0,0,1)),((0,0,1),(1,0,0),(0,1,0)),((0,0,-1),(1,0,0),(0,1,0))]
FA=[tuple(map(np.array,f)) for f in FACES]
def cube(N,H=1.0):
    def pos(f,i,j):
        n,u,v=FA[f]; return n+u*(-1+(2*i+1)/N)+v*(-1+(2*j+1)/N)
    def locate(p):
        a=int(np.argmax(np.abs(p))); f=[f for f in range(6) if FA[f][0][a]==math.copysign(1,p[a])][0]
        n,u,v=FA[f]; s=p@u; t=p@v
        i=min(N-1,max(0,int(math.floor((s+1)*N/2)))); j=min(N-1,max(0,int(math.floor((t+1)*N/2))))
        return f*N*N+j*N+i
    nb=[[] for _ in range(6*N*N)]; h=2/N
    for f in range(6):
      n,u,v=FA[f]
      for j in range(N):
        for i in range(N):
          idx=f*N*N+j*N+i; s=-1+(2*i+1)/N; t=-1+(2*j+1)/N
          for du,dv in [(1,0),(-1,0),(0,1),(0,-1)]:
            s2=s+du*h; t2=t+dv*h
            if abs(s2)<1 and abs(t2)<1: p=n+u*s2+v*t2
            elif s2>1: p=u+n*(1-(s2-1))+v*t
            elif s2<-1: p=-u+n*(1-(-1-s2))+v*t
            elif t2>1: p=v+n*(1-(t2-1))+u*s
            else: p=-v+n*(1-(-1-t2))+u*s
            nb[idx].append((locate(p),h*H,h*H))
    area=np.full(6*N*N,(h*H)**2); return area,nb,6*(2*H)**2
def plane(nx,ny,dx,bc,disc=False):
    if disc:
        d=nx;R=d*dx/2;cells={}
        for j in range(d):
            for i in range(d):
                x=(i+.5-d/2)*dx;y=(j+.5-d/2)*dx
                if x*x+y*y<=R*R: cells[(i,j)]=len(cells)
        nxx,nyy=d,d
    else:
        cells={(i,j):j*nx+i for j in range(ny) for i in range(nx)};nxx,nyy=nx,ny
    nb=[[] for _ in cells]
    for (i,j),k in cells.items():
        for di,dj in [(1,0),(-1,0),(0,1),(0,-1)]:
            a,b=i+di,j+dj
            if bc=='wrap': a%=nxx;b%=nyy
            if (a,b) in cells: nb[k].append((cells[(a,b)],dx,dx))
    area=np.full(len(cells),dx*dx)
    ana=math.pi*(nx*dx/2)**2 if disc else nx*ny*dx*dx
    return area,nb,ana
def check(name,area,nb,ana):
    asym=0
    d={(i,w):l for i in range(len(nb)) for (w,l,_) in nb[i]}
    for (i,w),l in d.items():
        if (w,i) not in d or abs(d[(w,i)]-l)>1e-9*l: asym+=1
    assert len(set((i,w) for (i,w) in d))==sum(len(x) for x in nb),"dup nbrs"
    # diffusion
    n=len(nb); rng=np.random.default_rng(1); T=rng.random(n); tot0=T@area
    dt=0.5*min(area[i]/sum(l/dd for _,l,dd in nb[i]) for i in range(n))
    for _ in range(40000):
        dT=np.array([sum(l/dd*(T[w]-T[i]) for w,l,dd in nb[i])/area[i] for i in range(n)]) if n<300 else None
        if dT is None: break
        T=T+dt*dT
    print(f"{name:14s} n={n:6d} area_err={abs(area.sum()-ana)/ana:.2e} asym={asym} deg={sorted(set(len(x) for x in nb))} "
          +(f"cons_err={abs(T@area-tot0)/tot0:.1e} spread={T.max()-T.min():.1e}" if n<300 else ""))
check("sphere L2",*sphere(2))
check("sphere L4",*sphere(4,6371e3))
check("cube N=4",*cube(4)); check("cube N=1",*cube(1)); check("cube N=20",*cube(20,3e3))
check("rect wrap",*plane(12,10,1.,'wrap')); check("rect wall",*plane(12,10,1.,'wall'))
check("disc 20",*plane(20,20,1.,'wall',True)); check("disc 100",*plane(100,100,1.,'wall',True))
