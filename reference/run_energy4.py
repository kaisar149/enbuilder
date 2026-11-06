from energy_mirror import *
import energy_mirror as E
E.P.update(a_land=.22,a_water=.06,a_ice=.60,cloud=.20)
z=np.array([0,0,1.]);x=np.array([1.,0,0]); a=np.array([.3,.5,.8]);a/=np.linalg.norm(a)
def world(M): return np.where(M['up']@a>.4,100.,-3000.)
Ms=mesh('sphere',3,6.371e6)
print("sphere tilted, passes by (omega,tol):")
for om,tol in ((.5,1e-3),(.5,2e-3),(.5,3e-3)):
    ps=[]
    for s in (0,.125,.25,.5,.75):
        Q=insolation_dir(Ms,1361,x,z,math.radians(23.44),True,86400,s); ps.append(loop(Ms,Q,world(Ms),om=om,tol=tol)[0])
    print(" ",om,tol,ps)
Mc=mesh('cube',8,3e6); up=Mc['up']
for om,tol in ((.5,1e-3),(.5,2e-3)):
    Q=insolation_dir(Mc,1361,x,z,0.0,True,86400,0.0); it,T,f=loop(Mc,Q,world(Mc),om=om,tol=tol)
    cap=abs(up[:,2])>.9; belt=abs(up[:,2])<.3
    print("cube",om,tol,"passes",it,"cap",T[cap].mean().round(1),"belt",T[belt].mean().round(1),"range",T.min().round(1),T.max().round(1))
N=8;same=[];cross=[]
for i in range(Mc['n']):
    for j,l,d in Mc['nb'][i]: (same if i//(N*N)==j//(N*N) else cross).append(abs(T[i]-T[j]))
print("mean|dT| same",np.mean(same).round(2),"cross",np.mean(cross).round(2))
