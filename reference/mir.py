import numpy as np, sys
def ico():
    t=(1+5**.5)/2
    v=np.array([[-1,t,0],[1,t,0],[-1,-t,0],[1,-t,0],[0,-1,t],[0,1,t],[0,-1,-t],[0,1,-t],[t,0,-1],[t,0,1],[-t,0,-1],[-t,0,1]],float)
    v/=np.linalg.norm(v,axis=1)[:,None]
    f=[[0,11,5],[0,5,1],[0,1,7],[0,7,10],[0,10,11],[1,5,9],[5,11,4],[11,10,2],[10,7,6],[7,1,8],[3,9,4],[3,4,2],[3,2,6],[3,6,8],[3,8,9],[4,9,5],[2,4,11],[6,2,10],[8,6,7],[9,8,1]]
    return list(v),f
def arc(a,b): return np.arctan2(np.linalg.norm(np.cross(a,b)),a@b)
def stri(a,b,c):
    return 2*np.arctan2(abs(a@np.cross(b,c)),1+a@b+b@c+c@a)
def sphere(L,R=6.371e6,rot=None):
    verts,faces=ico()
    for _ in range(L):
        cache={};nf=[]
        def mid(a,b):
            k=(min(a,b),max(a,b))
            if k in cache:return cache[k]
            m=verts[a]+verts[b];m/=np.linalg.norm(m);verts.append(m);cache[k]=len(verts)-1;return cache[k]
        for a,b,c in faces:
            ab,bc,ca=mid(a,b),mid(b,c),mid(c,a)
            nf+=[[a,ab,ca],[b,bc,ab],[c,ca,bc],[ab,bc,ca]]
        faces=nf
    V=np.array(verts)
    if rot is not None: V=V@rot.T
    n=len(V)
    tc=np.array([(V[a]+V[b]+V[c])/np.linalg.norm(V[a]+V[b]+V[c]) for a,b,c in faces])
    inc=[[] for _ in range(n)]
    for ti,f in enumerate(faces):
        for v in f: inc[v].append(ti)
    adj=[];el=[];ed=[];area=np.zeros(n)
    for v in range(n):
        p=V[v]; h=np.array([1,0,0.]) if abs(p[0])<.9 else np.array([0,1,0.])
        e1=np.cross(p,h);e1/=np.linalg.norm(e1);e2=np.cross(p,e1)
        order=sorted(inc[v],key=lambda t:np.arctan2(tc[t]@e2,tc[t]@e1))
        m=len(order);nb=[]
        for k in range(m):
            t0=order[k];t1=order[(k+1)%m]
            w=[x for x in faces[t0] if x!=v and x in faces[t1]][0]
            area[v]+=stri(p,tc[t0],tc[t1])
            nb.append((w,arc(tc[t0],tc[t1])*R,arc(p,V[w])*R))
        nb.sort()
        adj.append([x[0] for x in nb]);el.append([x[1] for x in nb]);ed.append([x[2] for x in nb])
    return V*R,V,area*R*R,adj,el,ed
def frames(up,axis=np.array([0,0,1.])):
    north=axis-np.outer(up@axis,axis*0+1)*0
    north=axis[None,:]-(up@axis)[:,None]*up
    nn=np.linalg.norm(north,axis=1)
    bad=nn<1e-9
    north[bad]=np.array([1,0,0.])-(up[bad]@np.array([1,0,0.]))[:,None]*up[bad]
    north/=np.linalg.norm(north,axis=1)[:,None]
    return north,np.cross(north,up)
def curlop(C,up,area,adj,el,ed):
    out=[]
    for i in range(len(C)):
        cs=[]
        for j,l,d in zip(adj[i],el[i],ed[i]):
            v=C[j]-C[i]; v=v-up[i]*(v@up[i]); n=v/np.linalg.norm(v)
            t=np.cross(up[i],n); cs.append(0.5*t*l/area[i])
        out.append(cs)
    return out
def apply(op,adj,v):
    return np.array([sum(c@(v[j]-v[i]) for c,j in zip(op[i],adj[i])) for i in range(len(adj))])
def gradop(C,up,north,east,adj,ed):
    out=[]
    for i in range(len(C)):
        items=[];sxx=sxy=syy=0
        for j,d in zip(adj[i],ed[i]):
            v=C[j]-C[i];v=v-up[i]*(v@up[i]);l=np.linalg.norm(v);dd=v*d/l
            dx,dy=dd@east[i],dd@north[i];w=1/d**2
            sxx+=w*dx*dx;sxy+=w*dx*dy;syy+=w*dy*dy;items.append((dx,dy,w))
        e=1e-6*(sxx+syy);sxx+=e;syy+=e;det=sxx*syy-sxy**2
        cs=[]
        for dx,dy,w in items:
            cx=w*(syy*dx-sxy*dy)/det;cy=w*(-sxy*dx+sxx*dy)/det
            cs.append(east[i]*cx+north[i]*cy)
        out.append(cs)
    return out
def rotm(ax,a):
    ax=np.array(ax,float);ax/=np.linalg.norm(ax)
    K=np.array([[0,-ax[2],ax[1]],[ax[2],0,-ax[0]],[-ax[1],ax[0],0]])
    return np.eye(3)+np.sin(a)*K+(1-np.cos(a))*K@K
if __name__=="__main__":
    R=6.371e6;Om=7.292e-5
    for rot in [None,rotm([1,2,3],0.7)]:
      print("rot",rot is not None)
      errs=[]
      for L in [3,4,5]:
        C,up,area,adj,el,ed=sphere(L,R,rot)
        north,east=frames(up)
        op=curlop(C,up,area,adj,el,ed)
        # solid body
        tau=np.cross(np.array([0,0,Om]),C)
        cu=apply(op,adj,tau)
        exp=2*Om*up[:,2]
        print(L,"solid err/2om",np.abs(cu-exp).max()/(2*Om))
        # grad field phi = A*x*z/R, A=1
        g=np.stack([C[:,2],0*C[:,0],C[:,0]],1)/R
        g=g-(g*up).sum(1)[:,None]*up
        cg=apply(op,adj,g)
        sc=np.abs(g).max()/R
        e=np.abs(cg).max()/sc;errs.append(e);print(L,"curlgrad max/(|g|/R)",e, "rms",np.sqrt((cg**2).mean())/sc)
        # beta
        gop=gradop(C,up,north,east,adj,ed)
        f=2*Om*up[:,2]
        gr=np.array([sum(c*(f[j]-f[i]) for c,j in zip(gop[i],adj[i])) for i in range(len(f))])
        beta=(gr*north).sum(1)
        lat=np.arcsin(up[:,2]);bt=2*Om*np.cos(lat)/R
        m=np.abs(lat)<np.radians(60)
        print(L,"beta rel err (<60)",np.abs(beta[m]/bt[m]-1).max(),"beta min lat<80",beta[np.abs(lat)<np.radians(80)].min()>0, "all-cell abs err/(2Om/R)",np.abs(beta-bt).max()/(2*Om/R))
