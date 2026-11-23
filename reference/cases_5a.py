"""Hand cases of tests/part5.rs checked against mirror_5a.py. Run: python cases_5a.py"""
from mirror_5a import *
import os
def kc(tc,pm): return koppen([x+K for x in tc],[m*12/YEAR for m in pm])
C=[([26,27,26,27],[250,200,220,240],"Af"),([26,27,26,27],[400,20,60,420],"Am"),([25,27,25,27],[5,200,5,200],"Aw"),([20,32,20,32],[5,10,5,10],"BWh"),
([20,30,20,30],[20,50,20,50],"BSh"),([-5,20,-5,20],[8]*4,"BWk"),([-2,22,-2,22],[15,25,15,25],"BSk"),([5,15,6,16],[60,70,60,70],"Cfb"),
([10,25,12,24],[90,5,95,5],"Csa"),([8,26,9,25],[5,200,6,180],"Cwa"),([-12,14,-10,15],[40,70,40,70],"Dfb"),([-20,12,-18,2],[30,50,30,50],"Dfc"),
([-15,24,-14,23],[5,150,6,140],"Dwa"),([-15,6,-14,5],[30]*4,"ET"),([-30,-5,-28,-6],[10]*4,"EF")]
bad=0
for t,p,e in C:
    g=kc(t,p)
    if g!=e: bad+=1; print("MISMATCH",t,p,"expected",e,"got",g)
print("koppen hand cases:",len(C)-bad,"/",len(C))
print("1 season Af",koppen([27+K],[2400/YEAR]),"BWh",koppen([30+K],[100/YEAR]),"12 seasons",koppen([27+K]*12,[200*12/YEAR]*12))
r=normals([x+K for x in [0,10,20,10]],[2/86400]*4)
print("normals1",r["clim_t_mean_c"],r["clim_p_annual"],r["clim_months_warm"],r["clim_gdd"],r["clim_pet"],r["clim_aridity"],"(want 10,730.5,3,2282.8125,559.7295,1.3051)")
r=normals([x+K for x in [-5,15,25,5]],[x/86400 for x in [1,2,6,3]])
print("normals2",r["clim_p_annual"],r["clim_p_dry"],r["clim_p_wet"],r["clim_p_summer_frac"],r["clim_months_warm"],r["clim_gdd"],r["clim_pet"],"(want 1095.75,365.25,2191.5,.6667,6,2739.375,659.6818)")
t=[10,25,12,24];p=[90,5,95,5];rot=lambda a:[a[2],a[3],a[0],a[1]]
print("hemisphere independent:",kc(t,p)==kc(rot(t),rot(p)))
# table edit test
here=os.path.dirname(__file__); txt=open(os.path.join(here,"..","data","earth_biomes.txt")).read()
def shares(text):
    tab=parse_table(text); names=[b["name"] for b in tab]+["u"]; rows=[r for r in world() if r[1]["ocean_mask"]<.5]; c={}
    for la,nm,k in rows: n=names[classify(tab,lambda f:nm[f])]; c[n]=c.get(n,0)+1
    return {k:v/len(rows) for k,v in c.items()}
a=shares(txt); b=shares(txt.replace("clim_aridity < 0.2\n","clim_aridity < 0.4\n"))
print("hot_desert",a.get("hot_desert"),"->",b.get("hot_desert"),"| thorn_scrub",a.get("thorn_scrub"),"->",b.get("thorn_scrub"))
