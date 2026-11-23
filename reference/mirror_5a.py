"""Python mirror of the 5a climate classification (biomes.rs) and of the analytic Earth-like world in tests/part5.rs.
Run: python mirror_5a.py   (needs numpy). Prints land shares per biome and the acceptance-band numbers."""
import math, sys, re, os
import numpy as np
K=273.15; YEAR=3.15576e7
CODES=["Ocean","Af","Am","Aw","BWh","BWk","BSh","BSk","Csa","Csb","Csc","Cwa","Cwb","Cwc","Cfa","Cfb","Cfc","Dsa","Dsb","Dsc","Dsd","Dwa","Dwb","Dwc","Dwd","Dfa","Dfb","Dfc","Dfd","ET","EF"]
def warm_half(tc):
    n=len(tc); idx=sorted(range(n),key=lambda k:(-tc[k],k)); w=[False]*n
    for k in idx[:(n+1)//2]: w[k]=True
    return w
def normals(tk,pr,slope=0.09,exp=1.2):
    n=len(tk); tc=[t-K for t in tk]; pyr=[max(r,0)*YEAR for r in pr]
    pann=sum(pyr)/n; w=warm_half(tc); sw=sum(pyr[k] for k in range(n) if w[k]); sa=sum(pyr)
    sf=sw/sa if sa>0 else .5
    mw=12*sum(1 for x in tc if x>10)/n
    gdd=sum(max(x-5,0) for x in tc)/n*365.25
    pet=sum(slope*max(x,0)**exp for x in tc)/n*365.25
    return dict(clim_t_mean_c=sum(tc)/n,clim_t_cold_c=min(tc),clim_t_warm_c=max(tc),clim_t_range_k=max(tc)-min(tc),clim_p_annual=pann,
      clim_p_dry=min(pyr),clim_p_wet=max(pyr),clim_p_summer_frac=sf,clim_months_warm=mw,clim_gdd=gdd,clim_pet=pet,clim_aridity=pann/max(pet,1.0))
def koppen(tk,pr,year=YEAR):
    n=len(tk); tc=[t-K for t in tk]; pm=[max(r,0)*year/12 for r in pr]
    tann=sum(tc)/n; tcold=min(tc); twarm=max(tc); pann=sum(pm)*12/n; mw=12*sum(1 for x in tc if x>10)/n
    if twarm<10: return "ET" if twarm>0 else "EF"
    w=warm_half(tc); wi=[k for k in range(n) if w[k]]; ci=[k for k in range(n) if not w[k]] or list(range(n))
    tot=sum(pm); fs=0.5 if (n==1 or tot<=0) else sum(pm[k] for k in wi)/tot
    add=28 if fs>=.7 else (0 if 1-fs>=.7 else 14); pth=max(2*tann+add,0)
    if pann<10*pth: return ("BW" if pann<5*pth else "BS")+("h" if tann>=18 else "k")
    pmin=min(pm)
    if tcold>=18:
        if pmin>=60: return "Af"
        return "Am" if pann>=25*(100-pmin) else "Aw"
    first="C" if tcold>0 else "D"
    sd=min(pm[k] for k in wi); sw_=max(pm[k] for k in wi); wd=min(pm[k] for k in ci); ww=max(pm[k] for k in ci)
    s2="s" if (sd<40 and sd<ww/3) else ("w" if wd<sw_/10 else "f")
    s3="a" if twarm>=22 else ("b" if mw>=4 else ("d" if tcold<-38 else "c"))
    return first+s2+s3
def parse_table(text):
    biomes=[]
    for raw in text.splitlines():
        tok=[]
        for t in raw.split():
            if t.startswith('#'): break
            tok.append(t)
        if not tok: continue
        if tok[0]=="biome":
            b=dict(name=tok[1],albedo=.22,evap=1.0,rough=0.0,conds=[],color=tok[2:] and None)
            for kv in tok[2:]:
                k,v=kv.split('=')
                if k=="albedo": b["albedo"]=float(v)
                elif k=="evap": b["evap"]=float(v)
                elif k=="rough": b["rough"]=float(v)
            biomes.append(b)
        else:
            f,op,val=tok[0],tok[1],tok[2]
            if op==">=": lo,hi=float(val),math.inf
            elif op=="<": lo,hi=-math.inf,float(val)
            else:
                a,c=val.split(".."); lo,hi=float(a),float(c)
            biomes[-1]["conds"].append((f,lo,hi))
    return biomes
def classify(biomes,get):
    for i,b in enumerate(biomes):
        if all(lo<=get(f)<hi for f,lo,hi in b["conds"]): return i
    return len(biomes)
# ---- analytic world (same formulas as tests/part5.rs) ----
LAT_T=[(0,27),(15,26),(30,19),(45,9),(60,1),(75,-12),(90,-23)]
LAT_A=[(0,1.5),(15,3),(30,8),(45,13),(60,17),(75,19),(90,20)]
LAT_P=[(0,2400),(10,2200),(20,800),(30,300),(40,650),(50,800),(60,600),(75,250),(90,150)]
SEAS=[.125,.375,.625,.875]
def interp(tab,x):
    if x<=tab[0][0]: return tab[0][1]
    for a,b in zip(tab,tab[1:]):
        if x<=b[0]:
            f=(x-a[0])/(b[0]-a[0]); return a[1]+f*(b[1]-a[1])
    return tab[-1][1]
def analytic_cell(lat,lon):
    al=abs(lat); sg=0.0 if lat==0 else math.copysign(1,lat)
    tm=interp(LAT_T,al); amp=interp(LAT_A,al)*(1+.4*math.sin(2*lon)); pann=interp(LAT_P,al)*(1+.6*math.sin(3*lon))
    t=[];p=[]
    for s in SEAS:
        ph=math.sin(2*math.pi*s); t.append(tm+amp*ph*sg+K)
        if al<25: w=1+.8*sg*ph*(1-al/25)
        elif 30<=al<45: w=1-.7*sg*ph
        else: w=1.0
        p.append(max(pann*w,0)/YEAR)
    tot=sum(p)/4
    p=[v*(pann/YEAR)/tot for v in p]
    return t,p
def is_land(lat,lon): return math.sin(lon+.3)+.5*math.sin(3*lon)+.2*math.cos(2*lat)>.1
def fib(n):
    i=np.arange(n)+.5; z=1-2*i/n; lon=(math.pi*(1+5**.5)*i)%(2*math.pi)-math.pi
    return np.degrees(np.arcsin(z)),lon
def world(n=2562):
    lat,lon=fib(n); rows=[]
    for la,lo in zip(lat,lon):
        t,p=analytic_cell(la,lo); land=is_land(math.radians(la),lo)
        nm=normals(t,p); nm["ocean_mask"]=0.0 if land else 1.0; nm["glacier_mask"]=1.0 if (land and la>80) else 0.0
        rows.append((la,nm,koppen(t,p)))
    return rows
if __name__=="__main__":
    path=sys.argv[1] if len(sys.argv)>1 else os.path.join(os.path.dirname(__file__),"..","data","earth_biomes.txt")
    tab=parse_table(open(path).read()); names=[b["name"] for b in tab]+["unclassified"]
    rows=world(); area={}; land=[(la,nm,k,names[classify(tab,lambda f:nm[f])]) for la,nm,k in rows if nm["ocean_mask"]<.5]
    print("land cells",len(land))
    cnt={}
    for _,_,_,n in land: cnt[n]=cnt.get(n,0)+1
    for n,c in sorted(cnt.items(),key=lambda x:-x[1]): print(f"  {n:26s}{c:5d} {c/len(land):.3f}")
    def share(cells,pred): return sum(1 for c in cells if pred(c))/max(len(cells),1)
    eq=[c for c in land if abs(c[0])<10]; sub=[c for c in land if 20<=abs(c[0])<35]; pol=[c for c in land if abs(c[0])>70]; bor=[c for c in land if 50<=abs(c[0])<65]
    g=lambda c:{"A":1,"B":2,"C":3,"D":4,"E":5}[c[2][0]]
    print("band sizes",len(eq),len(sub),len(pol),len(bor))
    R=[("eq forest>=.7",share(eq,lambda c:c[3] in("tropical_rainforest","tropical_seasonal_forest"))),("eq koppenA>=.95",share(eq,lambda c:g(c)==1)),
       ("sub dry>=.2",share(sub,lambda c:c[3] in("hot_desert","cold_desert","thorn_scrub","steppe"))),("sub hot_desert>=.03",share(sub,lambda c:c[3]=="hot_desert")),
       ("sub rainforest<.02",share(sub,lambda c:c[3]=="tropical_rainforest")),("pol tundra/ice>=.9",share(pol,lambda c:c[3] in("tundra","polar_ice","glacier"))),
       ("pol koppenE>=.9",share(pol,lambda c:g(c)==5)),("bor taiga>=.55",share(bor,lambda c:c[3]=="taiga")),("bor koppenD>=.65",share(bor,lambda c:g(c)==4))]
    for k,v in R: print(f"  {k:24s}{v:.3f}")
    bad=[c for c in land if c[3]=="hot_desert" and not (c[1]["clim_aridity"]<.2 and c[1]["clim_t_mean_c"]>=18 and 15<abs(c[0])<40)]
    print("hot_desert violations",len(bad)); bad=[c for c in land if c[3]=="tropical_rainforest" and not(c[1]["clim_aridity"]>=1.1 and c[1]["clim_t_cold_c"]>=18)]; print("rainforest violations",len(bad))
    print("glacier lat<=80:",sum(1 for c in land if c[3]=="glacier" and c[0]<=80))
