"""Python mirror of src/vegetation.rs (run_vegetation_loop) on the toy climate of tests/part5b.rs. Run: python mirror_5b.py"""
import math, os
from mirror_5a import *
TABLE=parse_table(open(os.path.join(os.path.dirname(__file__),"..","data","earth_biomes.txt")).read())
NB=len(TABLE); NAMES=[b["name"] for b in TABLE]+["unclassified"]
N=32  # plane(8,4)
def toy_climate(veg, i, k_alb=20.0, g_evap=0.5):
    """cell i: 4 seasons of T (K) and P (kg/m2/s), same formulas as toy_climate() in tests/part5b.rs"""
    alb,ev,_=veg
    base=[24.0,28.0,24.0,28.0]
    t=[b-k_alb*(alb-0.22)+K for b in base]
    pann=(100.0+60.0*i)*(1.0+g_evap*(ev-1.0))
    p=[pann/YEAR]*4
    return t,p
def run(params=None, k_alb=20.0, g_evap=0.5, ocean=lambda i: i%8==7, verbose=True):
    P=dict(relax=.5,decay=.7,max_iters=12,tol_a=.005,tol_e=.02,tol_r=.5,base=(0.22,1.0,0.0)); 
    if params: P.update(params)
    veg=[list(P["base"]) for _ in range(N)]; prev=None; hist=[]; conv=False
    for k in range(P["max_iters"]):
        ids=[];tg=[]
        for i in range(N):
            t,p=toy_climate(veg[i],i,k_alb,g_evap); nm=normals(t,p); nm["ocean_mask"]=1.0 if ocean(i) else 0.0; nm["glacier_mask"]=0.0
            b=classify(TABLE,lambda f:nm[f]); ids.append(b)
            tg.append((TABLE[b]["albedo"],TABLE[b]["evap"],TABLE[b]["rough"]) if b<NB else (0.22,1.0,0.0))
        r=P["relax"]*P["decay"]**k; dm=[0,0,0]
        for i in range(N):
            if ocean(i): continue
            for f in range(3):
                st=r*(tg[i][f]-veg[i][f]); veg[i][f]+=st; dm[f]=max(dm[f],abs(st))
        land=[i for i in range(N) if not ocean(i)]
        ch=1.0 if prev is None else sum(1 for i in land if prev[i]!=ids[i])/len(land)
        hist.append((k,r,ch,dm[0],dm[1],dm[2],sum(veg[i][0] for i in land)/len(land)))
        prev=ids
        if dm[0]<=P["tol_a"] and dm[1]<=P["tol_e"] and dm[2]<=P["tol_r"]: conv=True; break
    if verbose:
        for h in hist: print("it %d r=%.4f changed=%.4f d_alb=%.5f d_ev=%.5f d_rough=%.4f mean_alb=%.5f"%h)
        print("converged",conv,"passes",len(hist)); print("biomes",[NAMES[b] for b in ids])
    return hist,conv,veg,ids
if __name__=="__main__":
    print("--- default"); h,c,v,ids=run()
    print("--- relax decay 1.0 (no guarantee)"); run(dict(decay=1.0))
    print("--- feedback off (k_alb=0,g_evap=0)"); run(k_alb=0,g_evap=0)
    print("--- max_iters 2"); run(dict(max_iters=2))
    # baseline (pass 0) climate vs converged climate for the driest land cell and a wet one
    for i in (0,3,6):
        b=toy_climate([0.22,1,0],i); c_=toy_climate(v[i],i)
        print("cell",i,"bare P=%.1f mm/yr"%(b[1][0]*YEAR),"vegetated P=%.1f"%(c_[1][0]*YEAR),"T0 bare %.2f veg %.2f"%(b[0][0]-K,c_[0][0]-K),"veg",[round(x,4) for x in v[i]],NAMES[ids[i]])
