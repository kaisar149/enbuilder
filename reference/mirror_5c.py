"""Hand numbers of tests/part5c.rs (formulas of src/derived.rs). Run: python mirror_5c.py"""
def ramp(x,lo,hi):
    return 0.0 if x!=x else min(max((x-lo)/(hi-lo),0.0),1.0)
def fert(ar,gdd,soil,a=(0.1,1.0),g=(300,2500),sref=0.1): return ramp(ar,*a)*ramp(gdd,*g)*(0.5+0.5*ramp(soil,0,sref))
def hab(f,tc,tw,ar): return min(ramp(tc,-35,-5),1-ramp(tw,32,45))*ramp(ar,0.05,0.4)*(0.4+0.6*f)
def trav(slope,canopy,tc,tw,gl): return 1+min(4*slope,4)+min(max(canopy/30,0),1)+ramp(-tc,10,30)+0.5*ramp(tw,35,45)+(2 if gl else 0)
print("ramp",ramp(5,0,10),ramp(-1,0,10),ramp(11,0,10),ramp(float('nan'),0,1))
print("fert full",fert(1.5,3000,0.1),"fert mid",fert(0.55,1400,0.05),"fert dry",fert(0.05,3000,0.1),"fert cold",fert(1.5,100,0.1))
print("hab best",hab(1.0,10,25,1.0),"hab harsh",hab(0.5,-20,40,0.2275),"hab arctic",hab(0.0,-40,5,1.0),"hab hot",hab(1.0,20,47,1.0))
print("trav",trav(0.1,15,-20,40,False),"glacier",trav(0.0,0,-30,2,True),"flat bare warm",trav(0,0,5,25,False),"steep",trav(2.0,0,5,25,False))
