from energy_mirror import *
import energy_mirror as E
E.P.update(a_land=.22,a_water=.06,a_ice=.60,cloud=.20)
z=np.array([0,0,1.]);x=np.array([1.,0,0])
Ms=mesh('sphere',3,6.371e6); up=Ms['up']
elev=np.where(up@np.array([.3,.5,.8])/np.linalg.norm([.3,.5,.8])>.4,100.,-3000.)
R={}
for s in (.25,.75):
    Q=insolation_dir(Ms,1361,x,z,math.radians(23.44),True,86400,s)
    it,T,f=loop(Ms,Q,elev,tol=1e-3); R[s]=T
    print("season",s,"passes",it,"N",T[up[:,2]>.7].mean().round(1),"S",T[up[:,2]<-.7].mean().round(1))
# plane, point sun above centre
Mp=mesh('plane',40,40,1e5,'wall',True); H=2e6
Q=insolation_point(Mp,1361,np.array([0,0,H]),H)
el=np.full(Mp['n'],100.)
it,T,f=loop(Mp,Q,el,tol=1e-3)
r=np.hypot(Mp['cen'][:,0],Mp['cen'][:,1]); bins=np.linspace(0,2e6,6)
means=[T[(r>=bins[k])&(r<bins[k+1])].mean() for k in range(5)]
print("plane passes",it,"Q centre/edge",Q[np.argmin(r)].round(0),Q[np.argmax(r)].round(0),"T bins",np.round(means,1),"center-edge",round(means[0]-means[-1],1))
# cube
Mc=mesh('cube',8,3e6); up=Mc['up']; elev=np.full(Mc['n'],100.)
Q=insolation_dir(Mc,1361,x,z,0.0,True,86400,0.0)
it,T,f=loop(Mc,Q,elev,tol=1e-3)
cap=abs(up[:,2])>.9; belt=(abs(up[:,2])<.3)
print("cube passes",it,"cap mean",T[cap].mean().round(1),"belt mean",T[belt].mean().round(1),"min/max",T.min().round(1),T.max().round(1))
n=Mc['n'];N=8;same=[];cross=[]
for i in range(n):
    for j,l,d in Mc['nb'][i]:
        (same if i//(N*N)==j//(N*N) else cross).append(abs(T[i]-T[j]))
print("mean|dT| same-face",np.mean(same).round(2),"cross-face",np.mean(cross).round(2),"max same",np.max(same).round(2),"max cross",np.max(cross).round(2))
# corner vs face-centre in cube: corners have up tilted
corner=np.argsort(-np.minimum.reduce([abs(up[:,0]),abs(up[:,1]),abs(up[:,2])]))[:3]
print("corner-ish T",T[corner].round(1))
