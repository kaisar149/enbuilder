from mir import *
def curlop3(C,up,north,east,area,adj,el,ed,correct=True):
    out=[]
    for i in range(len(C)):
        S=sum(l*d for l,d in zip(el[i],ed[i]))
        ds=[];cs=[];ws=[]
        for j,l,d in zip(adj[i],el[i],ed[i]):
            v=C[j]-C[i]; v=v-up[i]*(v@up[i]); ln=np.linalg.norm(v); n=v/ln
            dv=n*d; t=np.cross(up[i],n)
            ds.append([dv@east[i],dv@north[i]]); cs.append([2*l/S*(t@east[i]),2*l/S*(t@north[i])]); ws.append(1/d**2)
        ds=np.array(ds);cs=np.array(cs);ws=np.array(ws)
        if correct:
            B0=ds.T@cs                       # B_ab = sum d_a c_b
            S2=(ds*ws[:,None]).T@ds
            S2=S2+1e-6*np.trace(S2)*np.eye(2)
            T=np.array([[0,1.],[-1,0]])
            X=((T-B0).T)@np.linalg.inv(S2)
            cs=cs+ws[:,None]*(ds@X.T)
        out.append([east[i]*c[0]+north[i]*c[1] for c in cs])
    return out
R=6.371e6;Om=7.292e-5
for rot in [None,rotm([1,2,3],0.7)]:
  for L in [3,4,5]:
    C,up,area,adj,el,ed=sphere(L,R,rot)
    north,east=frames(up)
    op=curlop3(C,up,north,east,area,adj,el,ed)
    tau=np.cross(np.array([0,0,Om]),C)
    cu=apply(op,adj,tau);exp=2*Om*up[:,2]
    g=np.stack([C[:,2],0*C[:,0],C[:,0]],1)/R
    g=g-(g*up).sum(1)[:,None]*up
    cg=apply(op,adj,g);sc=np.abs(g).max()/R
    uni=apply(op,adj,np.tile(np.array([1.,2,3]),(len(C),1)))
    print(rot is not None,L,"solid",np.abs(cu-exp).max()/(2*Om),"curlgrad max",np.abs(cg).max()/sc,"rms",np.sqrt((cg**2).mean())/sc,"uniform",np.abs(uni).max()*R)
