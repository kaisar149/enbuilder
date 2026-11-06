"""NumPy mirror of src/moisture.rs constants: e_sat reference check and tropical rate."""
import numpy as np
def esat(T): tc=np.maximum(T,150.0)-273.15; return 611.2*np.exp(17.62*tc/(243.12+tc))
for tc,r in [(0,611),(20,2339),(30,4246)]: print(tc, round(float(esat(tc+273.15)),1), r, f"{abs(esat(tc+273.15)-r)/r:.2%}")
k,rh,w=1.8e-8,0.5,7.0
for T in (300,288,260): print(T, "mm/day", k*esat(T)*(1-rh)*(0.5+0.1*w)*86400)
