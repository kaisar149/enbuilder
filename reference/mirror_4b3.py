"""Python mirror of src/land_water.rs (Part 4b-3): the per-cell ledger, glacier flow, supply caps and the land-state moisture source.
   python mirror_4b3.py          -> randomized invariants + the hand numbers quoted in tests/part4b3.rs
No dependencies beyond numpy-free python (random + math)."""
import math, random

DEF = dict(soil_capacity_m=0.15, snow_T=273.15, snow_ramp_K=3.0, melt_T=273.15, melt_rate=4.6e-8, glacier_threshold_m=0.5,
           glacier_persist_s=31_557_600.0, snow_min_m=1e-3, glacier_flow_coeff=1e-5, glacier_flow_max_frac=0.5, glacier_mask_min_m=0.1,
           ice_density_ratio=0.917, rho_water=1000.0, step_s=2_592_000.0, snow_cover_ref_m=0.01)

def snow_fraction(t, st, ramp):
    if not ramp > 0: return 1.0 if t < st else 0.0
    return min(max((st + 0.5 * ramp - t) / ramp, 0.0), 1.0)

def cell_step(p, precip, evap, evap_sub, temp, soil, swe, ice, age):
    """ice in m of ice. returns dict like CellOut."""
    ts, rho, r = p['step_s'], p['rho_water'], p['ice_density_ratio']
    pr = max(precip, 0.0); e = max(evap, 0.0); es = min(max(evap_sub, 0.0), e); esoil = e - es
    sf = snow_fraction(temp, p['snow_T'], p['snow_ramp_K'])
    tot = pr * ts / rho; snowfall = sf * tot; rain = tot - snowfall
    ice0 = ice * r; sub = es * ts / rho
    sub_snow = min(sub, swe); sub_ice = min(sub - sub_snow, ice0); unsub = sub - sub_snow - sub_ice
    swe1 = swe + snowfall - sub_snow; ice1 = ice0 - sub_ice
    pm = p['melt_rate'] * max(temp - p['melt_T'], 0.0) * ts
    ms = min(pm, swe1); swe2 = swe1 - ms
    mi = min(pm - ms, ice1); ice2 = ice1 - mi
    s1 = soil + rain + ms + mi
    evs = esoil * ts / rho; evs_eff = min(evs, s1); unsoil = evs - evs_eff
    s2 = s1 - evs_eff
    q = max(s2 - p['soil_capacity_m'], 0.0); send = s2 - q
    a = age + ts if swe2 > p['snow_min_m'] else 0.0
    if a >= p['glacier_persist_s'] and swe2 > p['glacier_threshold_m']:
        swe3, ice3 = p['glacier_threshold_m'], ice2 + (swe2 - p['glacier_threshold_m'])
    else:
        swe3, ice3 = swe2, ice2
    return dict(soil=send, swe=swe3, ice=ice3 / r, age=a, runoff=q, snowfall=snowfall, rain=rain, melt_snow=ms, melt_ice=mi,
                sub=sub_snow + sub_ice, soil_evap=evs_eff, unsup=unsub + unsoil)

def glacier_flow(p, n, area, fd, z, ocean, edist, ice):
    """edist[c] = distance to receiver fd[c] (the Rust code looks it up in the mesh)."""
    out = list(ice); snout = [0.0] * n
    for c in range(n):
        if ocean[c] or not ice[c] > 0 or fd[c] < 0: continue
        r = fd[c]
        if r == c: continue
        drop = z[c] - z[r]
        if not drop > 0: continue
        phi = max(min(p['glacier_flow_coeff'] * ice[c] * (drop / edist[c]) * p['step_s'], p['glacier_flow_max_frac']), 0.0)
        moved = phi * ice[c]; out[c] -= moved
        if ocean[r]: snout[c] += moved * p['ice_density_ratio']
        else: out[r] += moved * area[c] / area[r]
    return out, snout

def supply(p, soil, swe, ice):
    s = max(soil, 0.0); w = max(swe, 0.0) + p['ice_density_ratio'] * max(ice, 0.0)
    return dict(avail=min(max(s / p['soil_capacity_m'], 0.0), 1.0), soil_cap=p['rho_water'] * s / p['step_s'],
                sub_cap=p['rho_water'] * w / p['step_s'], cover=w / (w + p['snow_cover_ref_m']))

def source_land(pot, ice_frac, avail, soil_cap, sub_cap, cover, sub_coeff=0.1, cap=1e-4, ocean=False):
    open_ = min(max(1 - ice_frac, 0.0), 1.0)
    if ocean: return min(max(pot * open_, 0.0), cap), 0.0
    a = min(max(avail, 0.0), 1.0)
    liq = min(min(max(a * pot * open_, 0.0), cap), max(soil_cap, 0.0))
    sub = max(min(min(sub_coeff * min(max(cover, 0.0), 1.0) * pot, max(sub_cap, 0.0)), cap - liq), 0.0)
    return liq + sub, sub

def random_world(rng, n):
    area = [1e8 * (0.5 + rng.random()) for _ in range(n)]
    z = [rng.uniform(-50, 3000) for _ in range(n)]
    ocean = [zz < 0 for zz in z]
    order = sorted(range(n), key=lambda c: -z[c])
    fd = [-1] * n; ed = [1e4] * n
    for k, c in enumerate(order):                      # receiver = a strictly lower cell if one exists
        lower = [d for d in order[k + 1:k + 6] if z[d] < z[c]]
        if lower: fd[c] = rng.choice(lower)
        ed[c] = 1e4 * (0.5 + rng.random())
    return area, z, ocean, fd, ed

def run_cycle(rng, p, n, steps, hot):
    area, z, ocean, fd, ed = random_world(rng, n)
    soil = [rng.uniform(0, 0.15) for _ in range(n)]; swe = [rng.choice([0, 0, 0.3, 2.0]) for _ in range(n)]
    ice = [rng.choice([0, 0, 5.0, 80.0]) for _ in range(n)]; age = [rng.choice([0, 3e7]) for _ in range(n)]
    worst = 0.0; worst_unsup = 0.0; total = {}
    for step in range(steps):
        temp = [rng.uniform(235, 235 + hot) for _ in range(n)]
        pot = [rng.uniform(0, 4e-5) for _ in range(n)]
        precip = [rng.uniform(0, 3e-4) * rng.choice([0, 1, 1]) for _ in range(n)]
        icef = [rng.random() * rng.choice([0, 1]) for _ in range(n)]
        out = []; P = E = R = 0.0; S0 = sum(area[c] * (soil[c] + swe[c] + p['ice_density_ratio'] * ice[c]) for c in range(n) if not ocean[c])
        newsoil = [0.0] * n; newswe = [0.0] * n; newice = [0.0] * n; newage = [0.0] * n; rwe = [0.0] * n; U = 0.0
        for c in range(n):
            if ocean[c]: continue
            s = supply(p, soil[c], swe[c], ice[c])
            e, es = source_land(pot[c], icef[c], s['avail'], s['soil_cap'], s['sub_cap'], s['cover'])
            o = cell_step(p, precip[c], e, es, temp[c], soil[c], swe[c], ice[c], age[c])
            newsoil[c], newswe[c], newice[c], newage[c], rwe[c] = o['soil'], o['swe'], o['ice'], o['age'], o['runoff']
            P += max(precip[c], 0) * area[c] * p['step_s'] / p['rho_water']; E += e * area[c] * p['step_s'] / p['rho_water']; U += o['unsup'] * area[c]
            assert 0 <= o['soil'] <= p['soil_capacity_m'] + 1e-15 and o['swe'] >= -1e-15 and o['ice'] >= -1e-15
        fl, sn = glacier_flow(p, n, area, fd, z, ocean, ed, newice)
        for c in range(n):
            if ocean[c]: continue
            R += (rwe[c] + sn[c]) * area[c]
        S1 = sum(area[c] * (newsoil[c] + newswe[c] + p['ice_density_ratio'] * fl[c]) for c in range(n) if not ocean[c])
        res = (S1 - S0) - (P - E - R)
        worst = max(worst, abs(res) / max(P, E, 1e-300)); worst_unsup = max(worst_unsup, U / max(P, E, 1e-300))
        soil, swe, ice, age = newsoil, newswe, fl, newage
    return worst, worst_unsup, sum(1 for c in range(n) if ice[c] > 0.1)

if __name__ == '__main__':
    rng = random.Random(4)
    p = dict(DEF)
    for hot, name in ((10, 'cold'), (60, 'mixed'), (80, 'hot')):
        w = [run_cycle(rng, p, 60, 14, hot) for _ in range(40)]
        print(f'{name:6s} closure worst {max(x[0] for x in w):.2e}  unsupplied worst {max(x[1] for x in w):.2e}  glacier cells (mean) {sum(x[2] for x in w)/len(w):.1f}')
    print('--- hand numbers (LandParams defaults)')
    def show(label, **kw):
        a = dict(precip=0.0, evap=0.0, evap_sub=0.0, temp=290.0, soil=0.0, swe=0.0, ice=0.0, age=0.0); a.update(kw)
        o = cell_step(p, **a); print(label, {k: round(v, 12) for k, v in o.items()})
    show('A fill      ', precip=1e-5, soil=0.05)
    show('B overflow  ', precip=1e-4, soil=0.05)
    show('C evap      ', evap=2e-6, soil=0.1)
    show('D snow      ', precip=1e-5, temp=263.15)
    show('E melt all  ', swe=0.05, temp=278.15, soil=0.05)
    show('F melt part ', swe=1.0, temp=275.15)
    show('G ice melts ', swe=0.1, ice=1.0, temp=278.15, soil=0.05)
    show('H sublim    ', swe=0.05, ice=0.1, evap=1e-5, evap_sub=1e-5, temp=260.0)
    show('H2 sub->ice ', swe=0.01, ice=0.1, evap=1e-5, evap_sub=1e-5, temp=260.0)
    show('H3 unsupp   ', evap=1e-5, evap_sub=1e-5, temp=260.0)
    show('I glacier   ', swe=0.9, age=3e7, temp=250.0)
    show('I2 too young', swe=0.9, age=1e6, temp=250.0)
    show('I3 below thr', swe=0.4, age=3e7, temp=250.0)
    show('J age reset ', swe=0.0, age=3e7, temp=250.0)
    show('K ramp half ', precip=1e-5, temp=273.15)
    # glacier flow chain: row of 6 cells, receiver = i-1, ocean cell 0, z = 100*i, ice 100 m in cell 3, k = 1e-8
    pf = dict(p); pf['glacier_flow_coeff'] = 1e-8
    n = 6; area = [1e8] * n; z = [100.0 * i for i in range(n)]; ocean = [i == 0 for i in range(n)]; fd = [-1] + [i - 1 for i in range(1, n)]; ed = [1e4] * n
    ice = [0, 0, 0, 100.0, 0, 0]; out, sn = glacier_flow(pf, n, area, fd, z, ocean, ed, ice)
    print('flow  chain', out, sn, 'volume', sum(a * b for a, b in zip(area, out)))
    ice = [0, 100.0, 0, 0, 0, 0]; out, sn = glacier_flow(pf, n, area, fd, z, ocean, ed, ice)
    print('flow  snout', out, sn)
    pf['glacier_flow_coeff'] = 1e-3; ice = [0, 0, 0, 100.0, 0, 0]; out, sn = glacier_flow(pf, n, area, fd, z, ocean, ed, ice)
    print('flow  capped', out, sn)
    area2 = [1e8, 1e8, 1e8, 4e8, 1e8, 1e8]; pf['glacier_flow_coeff'] = 1e-8; out, sn = glacier_flow(pf, n, area2, fd, z, ocean, ed, [0, 0, 0, 100.0, 0, 0])
    print('flow  unequal areas', out, 'volume', sum(a * b for a, b in zip(area2, out)))
    print('--- supply and source')
    s = supply(p, 0.075, 0.2, 10.0); print('supply', s)
    print('source land', source_land(2e-5, 0.0, 0.5, s['soil_cap'], s['sub_cap'], s['cover']), 'ocean', source_land(2e-5, 0.0, 1, 0, 0, 0, ocean=True))
    print('source soil-limited', source_land(2e-5, 0.0, 1.0, 1e-6, 0, 0.0))
    print('source sublimation', source_land(2e-5, 0.0, 0.0, 0.0, 1e-5, 0.9))
