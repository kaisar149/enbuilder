from energy_mirror import *
import energy_mirror as E
E.P.update(a_land=.22,a_water=.06,a_ice=.60,cloud=.20)
z=np.array([0,0,1.]);x=np.array([1.,0,0])
Ms=mesh('sphere',3,6.371e6); up=Ms['up']
elev=np.where(up@np.array([.3,.5,.8])/np.linalg.norm([.3,.5,.8])>.4,100.,-3000.)
print("land frac",np.average(elev>0,weights=Ms['area']).round(2))
for tilt in (0.0,23.44):
  for om,tol in ((.5,1e-4),(.5,1e-3),(.7,1e-3)):
    Q=insolation_dir(Ms,1361,x,z,math.radians(tilt),True,86400,0.0)
    it,T,f=loop(Ms,Q,elev,om=om,tol=tol)
    print(f"tilt{tilt} om={om} tol={tol}: passes={it} eq={T[abs(up[:,2])<.2].mean():.1f} pole={T[abs(up[:,2])>.9].mean():.1f} glob={np.average(T,weights=Ms['area']):.1f} ice={np.average(f,weights=Ms['area']):.2f}")
