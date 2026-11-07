"""Numpy mirror of Part 4a-2e: ocean_currents with the 4a-2e rules (wall pinning, waterworld = zero flow, H_mixed_m 20) on the sphere,
cube and plane, plus the diagnostics of src/ocean_validation.rs. Same geometry/operators as mirror_gyres.py (which reproduces the
recorded 4a-2d Rust numbers). The thresholds in tests/part4.rs are checked against this file:   python mirror_4a2e.py [sphere|plane|cube|tidal|all]
"""
import sys, numpy as np, scipy.sparse as sp, scipy.sparse.linalg as sla
from mir import sphere as _sphere, frames, gradop as _gradop_sphere
from mirror_gyres import build, curlop3 as _curlop3_sphere, R

OMEGA = 7.292e-5; RHO = 1025.0
DEF = dict(H=20.0, r=1e-5, clamp=3.0, rho=RHO, beta_on=True, slip=True, wall_pin=True)

# ---------------------------------------------------------------------------------------------------- worlds
def sphere_world(L, rot=None):
    return build(L, rot)

def plane_world(nx, ny, h=1e4, wrap=False):
    n = nx * ny; C = np.zeros((n, 3)); adj = []; el = []; ed = []; bl = np.zeros(n)
    for j in range(ny):
        for i in range(nx):
            k = j * nx + i; C[k] = [i * h, j * h, 0]
    for j in range(ny):
        for i in range(nx):
            nb = []
            for ii, jj in [(i - 1, j), (i + 1, j), (i, j - 1), (i, j + 1)]:
                if wrap: ii %= nx; jj %= ny
                if 0 <= ii < nx and 0 <= jj < ny: nb.append(jj * nx + ii)
                else: bl[j * nx + i] += h
            nb = sorted(nb)
            adj.append(nb); el.append([h] * len(nb)); ed.append([h] * len(nb))
    up = np.tile([0, 0, 1.], (n, 1)); north = np.tile([0, 1., 0], (n, 1)); east = np.tile([1., 0, 0], (n, 1))
    # centred coordinates like the Rust generator (y centred)
    C = C - C.mean(axis=0)
    return dict(C=C, up=up, area=np.full(n, h * h), adj=adj, el=el, ed=ed, north=north, east=east, n=n, nx=nx, ny=ny, h=h, bl=bl,
                wall=(not wrap), wrap=wrap)

def cube_world(n=14, hw=3.0e6):
    FACES = [([1, 0, 0], [0, 1, 0], [0, 0, 1]), ([-1, 0, 0], [0, 1, 0], [0, 0, 1]), ([0, 1, 0], [1, 0, 0], [0, 0, 1]),
             ([0, -1, 0], [1, 0, 0], [0, 0, 1]), ([0, 0, 1], [1, 0, 0], [0, 1, 0]), ([0, 0, -1], [1, 0, 0], [0, 1, 0])]
    FACES = [tuple(np.array(x, float) for x in f) for f in FACES]
    def locate(q):
        a = int(np.argmax(np.abs(q))); face = a * 2 + (0 if q[a] > 0 else 1); _, u, v = FACES[face]
        ci = int(np.clip(np.floor((q @ u + 1) * n / 2), 0, n - 1)); cj = int(np.clip(np.floor((q @ v + 1) * n / 2), 0, n - 1))
        return face * n * n + cj * n + ci
    h = 2.0 / n; cs = h * hw; N = 6 * n * n
    C = np.zeros((N, 3)); up = np.zeros((N, 3)); adj = []; el = []; ed = []
    for f in range(6):
        nrm, u, v = FACES[f]
        for j in range(n):
            for i in range(n):
                idx = f * n * n + j * n + i
                s = -1 + (2 * i + 1) / n; t = -1 + (2 * j + 1) / n
                q = nrm + s * u + t * v; C[idx] = q * hw; up[idx] = q / np.linalg.norm(q)
                nb = []
                for du, dv in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
                    s2 = s + du * h; t2 = t + dv * h
                    if abs(s2) < 1 and abs(t2) < 1: pt = nrm + s2 * u + t2 * v
                    elif s2 > 1: pt = u + (1 - (s2 - 1)) * nrm + t * v
                    elif s2 < -1: pt = -u + (1 - (-1 - s2)) * nrm + t * v
                    elif t2 > 1: pt = v + (1 - (t2 - 1)) * nrm + s * u
                    else: pt = -v + (1 - (-1 - t2)) * nrm + s * u
                    nb.append(locate(pt))
                nb = sorted(nb); adj.append(nb); el.append([cs] * len(nb)); ed.append([cs] * len(nb))
    north, east = frames(up)
    return dict(C=C, up=up, area=np.full(N, cs * cs), adj=adj, el=el, ed=ed, north=north, east=east, n=N, bl=np.zeros(N), wall=False, wrap=False, cube_n=n)

def dv(W, i, j, ed=None):
    v = W['C'][j] - W['C'][i]
    if W.get('wrap') and np.linalg.norm(v) > 1.5 * (W['h']): v = -v
    return v

# ---------------------------------------------------------------------------------------------------- operators (hooked copies)
def curlop(W):
    C, up, north, east, adj, el, ed = W['C'], W['up'], W['north'], W['east'], W['adj'], W['el'], W['ed']
    out = []
    for i in range(W['n']):
        S = sum(l * d for l, d in zip(el[i], ed[i])); ds = []; cs = []; ws = []
        for j, l, d in zip(adj[i], el[i], ed[i]):
            v = dv(W, i, j); v = v - up[i] * (v @ up[i]); nn = v / np.linalg.norm(v)
            dd = nn * d; t = np.cross(up[i], nn)
            ds.append([dd @ east[i], dd @ north[i]]); cs.append([2 * l / S * (t @ east[i]), 2 * l / S * (t @ north[i])]); ws.append(1 / d ** 2)
        ds = np.array(ds); cs = np.array(cs); ws = np.array(ws)
        B0 = ds.T @ cs; S2 = (ds * ws[:, None]).T @ ds; S2 = S2 + 1e-10 * np.trace(S2) * np.eye(2)
        T = np.array([[0, 1.], [-1, 0]]); X = ((T - B0).T) @ np.linalg.inv(S2); cs = cs + ws[:, None] * (ds @ X.T)
        out.append([east[i] * c[0] + north[i] * c[1] for c in cs])
    return out

def gradop(W):
    C, up, north, east, adj, ed = W['C'], W['up'], W['north'], W['east'], W['adj'], W['ed']
    out = []
    for i in range(W['n']):
        items = []; sxx = sxy = syy = 0
        for j, d in zip(adj[i], ed[i]):
            v = dv(W, i, j); v = v - up[i] * (v @ up[i]); l = np.linalg.norm(v); dd = v * d / l
            dx, dy = dd @ east[i], dd @ north[i]; w = 1 / d ** 2
            sxx += w * dx * dx; sxy += w * dx * dy; syy += w * dy * dy; items.append((dx, dy, w))
        e = 1e-6 * (sxx + syy); sxx += e; syy += e; det = sxx * syy - sxy ** 2; cs = []
        for dx, dy, w in items:
            cx = w * (syy * dx - sxy * dy) / det; cy = w * (-sxy * dx + sxx * dy) / det
            cs.append(east[i] * cx + north[i] * cy)
        out.append(cs)
    return out

def apply_g(W, gop, phi):
    return np.array([sum(c * (phi[j] - phi[i]) for c, j in zip(gop[i], W['adj'][i])) if W['adj'][i] else np.zeros(3) for i in range(W['n'])])

def curl_masked(W, op, v, ocean):
    out = np.zeros(W['n'])
    for i in np.where(ocean)[0]:
        out[i] = sum(c @ (v[j] - v[i]) for c, j in zip(op[i], W['adj'][i]) if ocean[j])
    return out

def beta_of(W, gop, f):
    g = apply_g(W, gop, f); return np.einsum('ij,ij->i', g, W['north'])

# ---------------------------------------------------------------------------------------------------- masks / effective ocean
def components(W, flag):
    comp = -np.ones(W['n'], int); k = 0
    for s in range(W['n']):
        if not flag[s] or comp[s] >= 0: continue
        comp[s] = k; st = [s]
        while st:
            i = st.pop()
            for j in W['adj'][i]:
                if flag[j] and comp[j] < 0: comp[j] = k; st.append(j)
        k += 1
    return comp

def effective(W, ocean, wall_pin=True):
    eff = ocean.copy(); pinned = 0
    if wall_pin and W.get('wall'):
        for i in range(W['n']):
            if eff[i] and W['bl'][i] > 0: eff[i] = False; pinned += 1
    comp = components(W, eff); unanch = 0
    for c in range(comp.max() + 1 if comp.max() >= 0 else 0):
        cells = np.where(comp == c)[0]
        if not any(any(not eff[j] for j in W['adj'][i]) for i in cells):
            eff[cells] = False; unanch += len(cells)
    return eff, pinned, unanch

def coast_hops(W, ocean):
    INF = 10 ** 9; h = np.full(W['n'], INF); q = []
    for i in range(W['n']):
        if ocean[i] and any(not ocean[j] for j in W['adj'][i]): h[i] = 0; q.append(i)
    k = 0
    while k < len(q):
        i = q[k]; k += 1
        for j in W['adj'][i]:
            if ocean[j] and h[j] == INF: h[j] = h[i] + 1; q.append(j)
    return h

# ---------------------------------------------------------------------------------------------------- solver
def solve(W, ocean, curl, beta, p):
    n = W['n']; cells = np.where(ocean)[0]; psi = np.zeros(n)
    if len(cells) == 0: return psi
    loc = -np.ones(n, int); loc[cells] = np.arange(len(cells))
    r = p['r']; denom = p['rho'] * p['H'] * r
    c = np.zeros((n, 3))
    if p['beta_on']: c[cells] = -(beta[cells] / r)[:, None] * W['east'][cells]
    rows = []; cols = []; vals = []; rhs = np.zeros(len(cells))
    for l, i in enumerate(cells):
        diag = 0.0
        for j, e, d in zip(W['adj'][i], W['el'][i], W['ed'][i]):
            co = e / d; v = 0.0
            if p['beta_on']:
                dd = dv(W, i, j); upm = W['up'][i] + W['up'][j]; upm = upm / np.linalg.norm(upm)
                dd = dd - upm * (dd @ upm); dl = np.linalg.norm(dd)
                if dl > 1e-12 * d: v = ((c[i] + c[j]) * 0.5 @ dd) / dl * e
            inflow = max(-v, 0.0)
            diag += co + inflow
            if ocean[j]: rows.append(l); cols.append(loc[j]); vals.append(-(co + inflow))
        rows.append(l); cols.append(l); vals.append(diag)
        rhs[l] = W['area'][i] * (-curl[i] / denom)
    K = sp.csr_matrix((vals, (rows, cols)), shape=(len(cells), len(cells)))
    psi[cells] = sla.spsolve(K.tocsc(), rhs)
    return psi

def coast_S(W, ocean):
    S = np.zeros((W['n'], 3)); Ls = np.zeros(W['n'])
    for i in np.where(ocean)[0]:
        for j, e in zip(W['adj'][i], W['el'][i]):
            if not ocean[j]:
                v = dv(W, i, j); v = v - W['up'][i] * (v @ W['up'][i]); v /= np.linalg.norm(v); S[i] += e * v; Ls[i] += e
    return S, Ls

def current(W, gop, psi, ocean, p):
    g = apply_g(W, gop, psi); u = np.zeros((W['n'], 3))
    for i in np.where(ocean)[0]:
        gt = g[i] - W['up'][i] * (g[i] @ W['up'][i]); u[i] = np.cross(W['up'][i], gt)
    if p['slip']:
        S, Ls = coast_S(W, ocean)
        for i in np.where(ocean)[0]:
            s = np.linalg.norm(S[i])
            if Ls[i] > 0 and s > 1e-6 * Ls[i]: sh = S[i] / s; u[i] -= (u[i] @ sh) * sh
    sp_ = np.linalg.norm(u, axis=1); big = sp_ > p['clamp']; nclamp = int(big.sum())
    u[big] *= (p['clamp'] / sp_[big])[:, None]
    return u, nclamp

class Case:
    def __init__(self, W, land, tau, beta, **over):
        self.W = W; self.p = dict(DEF); self.p.update(over)
        n = W['n']; self.mask = ~land
        self.op = curlop(W); self.gop = gradop(W)
        self.curl = curl_masked(W, self.op, tau, self.mask); self.beta = beta
        self.tau = tau
    def run(self, **over):
        p = dict(self.p); p.update(over); W = self.W
        eff, pinned, unanch = effective(W, self.mask, p['wall_pin'])
        psi = solve(W, eff, self.curl, self.beta, p)
        u, ncl = current(W, self.gop, psi, eff, p)
        self.eff = eff; self.pinned = pinned; self.unanch = unanch; self.psi = psi; self.u = u; self.ncl = ncl
        self.sp = np.linalg.norm(u, axis=1); return self

def lat_of(W): return np.arcsin(np.clip(W['up'][:, 2], -1, 1))
def lat_deg(W): return np.degrees(lat_of(W))
def earth_beta(W, om=OMEGA, gop=None):
    gop = gop or gradop(W); return beta_of(W, gop, 2 * om * W['up'][:, 2])
def zonal_stress(W, tau0):
    return -tau0 * np.cos(3 * lat_of(W))[:, None] * W['east']

EARTH_CAPS = [([1, .2, .3], 42), ([-.8, .5, -.1], 36), ([.1, -.9, -.4], 30), ([0, 0, -1], 20)]
def earth_land(W):
    land = np.zeros(W['n'], bool)
    for ax, th in EARTH_CAPS:
        a = np.array(ax, float); a /= np.linalg.norm(a); land |= (W['up'] @ a) > np.cos(np.radians(th))
    return land

# ---------------------------------------------------------------------------------------------------- diagnostics (same definitions as ocean_validation.rs)
def speed_report(W, ocean, sp_, lo, hi):
    hops = coast_hops(W, ocean); lat = np.abs(lat_deg(W)); INF = 10 ** 9
    sel = ocean & (hops < INF) & (lat >= lo) & (lat <= hi)
    b = sel & (hops <= 2); i = sel & (hops > 2)
    return dict(boundary_peak=sp_[b].max() if b.any() else 0.0, interior_median=float(np.sort(sp_[i])[i.sum() // 2]) if i.any() else 0.0, nb=int(b.sum()), ni=int(i.sum()))

def seam_spikes(W, ocean, sp_, cells, limit=0.2, floor=0.05):
    hops = coast_hops(W, ocean); INF = 10 ** 9; worst = 0.0; ev = 0; over = 0
    for i in cells:
        if not ocean[i] or hops[i] == INF or hops[i] < 2: continue
        nb = W['adj'][i]
        if any(not ocean[j] for j in nb): continue
        m = np.mean([sp_[j] for j in nb])
        if not m >= floor: continue
        s = abs(sp_[i] - m) / m; ev += 1; worst = max(worst, s); over += s > limit
    return dict(worst=worst, evaluable=ev, candidates=len(cells), over=over)

def nonzonal_fraction(W, mask, f, band):
    nb = int(np.ceil(180 / band)) + 1
    b = np.minimum(np.floor((lat_deg(W) + 90) / band).astype(int), nb - 1)
    num = den = 0.0
    sa = np.zeros(nb); sf = np.zeros(nb)
    for i in np.where(mask)[0]: sa[b[i]] += W['area'][i]; sf[b[i]] += W['area'][i] * f[i]
    for i in np.where(mask)[0]:
        zm = sf[b[i]] / sa[b[i]]; num += W['area'][i] * (f[i] - zm) ** 2; den += W['area'][i] * f[i] ** 2
    return num / den if den > 0 else 0.0

def vorticity_balance(W, op, ocean, u, beta, curl_tau, r, rho, h, min_hops):
    zeta = curl_masked(W, op, u, ocean); hops = coast_hops(W, ocean); INF = 10 ** 9
    v = np.einsum('ij,ij->i', u, W['north']); sel = ocean & (hops != INF) & (hops >= min_hops)
    F = curl_tau / (rho * h); res = r * zeta + beta * v - F
    return float(np.sqrt((res[sel] ** 2).sum() / (F[sel] ** 2).sum())), int(sel.sum())

def pentagons(W): return [i for i in range(W['n']) if len(W['adj'][i]) == 5]
def cross_face(W):
    n = W['cube_n']; pf = n * n; return [i for i in range(W['n']) if any(j // pf != i // pf for j in W['adj'][i])]

# ---------------------------------------------------------------------------------------------------- experiments
def exp_sphere(levels=(4, 5)):
    for L in levels:
        W = sphere_world(L); land = earth_land(W); tau = zonal_stress(W, 0.1); beta = earth_beta(W)
        c = Case(W, land, tau, beta).run()
        frac = W['area'][c.mask].sum() / W['area'].sum()
        rep = speed_report(W, c.eff, c.sp, 10, 60)
        print(f"[Earth L{L}] ocean frac {frac:.3f} clamped {c.ncl} unanch {c.unanch} boundary peak {rep['boundary_peak']:.3f} (n {rep['nb']}) interior median {rep['interior_median']:.3f} (n {rep['ni']})")
        hops = coast_hops(W, c.eff); lat = lat_deg(W); INF = 10 ** 9
        for lo, hi, sg in [(10, 40, 1), (-40, -10, -1)]:
            m = c.eff & (hops >= 3) & (hops != INF) & (lat > lo) & (lat < hi); print(f"   band {lo}..{hi}: mean psi {c.psi[m].mean():.3e} (n {m.sum()})")
        S, Ls = coast_S(W, c.eff); ws = wn = es = en = 0
        for i in range(W['n']):
            l = abs(lat[i])
            if not Ls[i] > 0 or l < 10 or l > 50: continue
            ew = S[i] @ W['east'][i]; sl = np.linalg.norm(S[i])
            if ew < -0.5 * sl: ws += c.sp[i]; wn += 1
            elif ew > 0.5 * sl: es += c.sp[i]; en += 1
        print(f"   west mean {ws/max(wn,1):.3f} (n {wn}) east mean {es/max(en,1):.3f} (n {en}) ratio {(ws/max(wn,1))/(es/max(en,1)):.2f}")
        pent = pentagons(W); rep = seam_spikes(W, c.eff, c.sp, pent); print("   pentagons", rep)
        hex_ = [i for i in range(W['n']) if len(W['adj'][i]) == 6]; rep = seam_spikes(W, c.eff, c.sp, hex_); print("   hexagons (same detector)", rep)
        F = np.array([c.u[i] @ S[i] for i in range(W['n'])]); m = Ls > 0; print("   coast flux max |F|/(L umax)", np.abs(F[m] / Ls[m]).max() / c.sp.max())
    return c

def exp_waterworld():
    W = sphere_world(4); n = W['n']; c = Case(W, np.zeros(n, bool), zonal_stress(W, .1), earth_beta(W)).run()
    print("[waterworld] unanchored", c.unanch, "max |psi|", abs(c.psi).max(), "max speed", c.sp.max())
    land = np.zeros(n, bool); land[0] = True; c = Case(W, land, zonal_stress(W, .1), earth_beta(W)).run()
    print("[one island L4] unanch", c.unanch, "clamped", c.ncl, "max speed", c.sp.max(), "max|psi|", abs(c.psi).max())

def basin_land(NX, NY, xs, ys):
    land = np.ones(NX * NY, bool)
    for y in ys:
        for x in xs: land[y * NX + x % NX] = False
    return land

def plane_case(boundary, land, NX=60, NY=40):
    W = plane_world(NX, NY, wrap=(boundary == 'wrap')); ly = NY * 1e4
    tau = np.zeros((W['n'], 3)); tau[:, 0] = -0.005 * np.cos(2 * np.pi * W['C'][:, 1] / ly)
    return Case(W, land, tau, np.full(W['n'], 2.3e-11), r=7e-7, clamp=1e9)

def exp_plane():
    NX, NY = 60, 40
    land = basin_land(NX, NY, list(range(8, 52)), range(6, 34))
    cw = plane_case('wall', land).run(); cr = plane_case('wrap', land).run()
    print("[plane] wall vs wrap psi diff", abs(cw.psi - cr.psi).max(), "scale", abs(cw.psi).max(), "pinned", cw.pinned, "unanch", cw.unanch)
    sp_ = cw.sp
    pk = lambda x0, x1: max(sp_[y * NX + x] for y in range(6, 34) for x in range(x0, x1))
    print(f"   west peak {pk(8,16):.3f} east peak {pk(44,52):.3f} ratio {pk(8,16)/pk(44,52):.2f}")
    W = cw.W; S, Ls = coast_S(W, cw.eff); F = np.array([cw.u[i] @ S[i] for i in range(W['n'])]); m = Ls > 0
    print("   coast flux", np.abs(F[m] / Ls[m]).max() / sp_.max())
    hops = coast_hops(W, cw.eff); INF = 10 ** 9; deep = (hops >= 3) & (hops != INF)
    gl = np.array([[0] * 3] * 1)
    d = np.zeros(W['n'])
    for i in np.where(deep)[0]: d[i] = sum(c @ (cw.u[j] - cw.u[i]) for c, j in zip(cw.gop[i], W['adj'][i]))
    print("   interior div * h / umax", abs(d[deep]).max() * 1e4 / sp_.max())
    row = [20 * NX + x for x in range(NX)]; net = sum(cw.u[k][1] for k in row); ab = sum(abs(cw.u[k][1]) for k in row); print("   section net/abs", abs(net) / ab)
    # straddle
    st = list(range(50, 60)) + list(range(0, 10)); ce = list(range(20, 40))
    a = plane_case('wrap', basin_land(NX, NY, st, range(6, 34))).run(); b = plane_case('wrap', basin_land(NX, NY, ce, range(6, 34))).run()
    sh = lambda k: (k // NX) * NX + ((k % NX) + 30) % NX
    print("[straddle] max diff", max(abs(a.sp[k] - b.sp[sh(k)]) for k in range(NX * NY)) / a.sp.max(), "crossing", sum(1 for k in range(NX * NY) if (k % NX < 10 or k % NX >= 50) and a.sp[k] > 0))
    # wall ring
    ring = np.array([not (0 < k % NX < NX - 1 and 0 < k // NX < NY - 1) for k in range(NX * NY)])
    W = plane_world(NX, NY, wrap=False); ly = NY * 1e4; tau = np.zeros((W['n'], 3)); tau[:, 0] = -0.005 * np.cos(2 * np.pi * W['C'][:, 1] / ly)
    A = Case(W, np.zeros(NX * NY, bool), tau, np.full(NX * NY, 2.3e-11), r=7e-7, clamp=1e9); B = Case(W, ring, tau, np.full(NX * NY, 2.3e-11), r=7e-7, clamp=1e9)
    interior = ~ring; A.curl = curl_masked(W, A.op, tau, interior); B.curl = A.curl
    A.run(); B.run(); print("[wall ring] pinned", A.pinned, "psi diff", abs(A.psi - B.psi).max() / abs(B.psi).max(), "max speed", B.sp.max())

def exp_cube():
    W = cube_world(14); n = 14
    land = np.array([(k // (n * n)) == 5 or ((k // (n * n)) == 0 and abs((k % (n * n)) % n - 6.5) <= 2 and abs((k % (n * n)) // n - 6.5) <= 2) for k in range(W['n'])])
    c = Case(W, land, zonal_stress(W, .1), earth_beta(W)).run()
    print("[cube] ocean", c.mask.sum(), "unanch", c.unanch, "clamped", c.ncl, "max speed", c.sp.max())
    print("   per face max", [round(c.sp[f*196:(f+1)*196].max(), 3) for f in range(6)])
    cf = cross_face(W); print("   seam", seam_spikes(W, c.eff, c.sp, cf))
    S, Ls = coast_S(W, c.eff); F = np.array([c.u[i] @ S[i] for i in range(W['n'])]); m = Ls > 0; print("   coast flux", np.abs(F[m] / Ls[m]).max() / c.sp.max())

def tidal_stress(W, tau0):
    s = np.array([1., 0, 0]); al = np.radians(30); out = np.zeros((W['n'], 3))
    for i in range(W['n']):
        up = W['up'][i]; c = up @ s; raw = s - up * c; nn = np.linalg.norm(raw)
        if nn < 1e-9: continue
        ts = raw / nn; sth = np.sqrt(max(1 - c * c, 0)); sg = np.sign(up[2]); rot = np.cross(up, ts)
        out[i] = tau0 * sth * (np.cos(al) * ts + sg * np.sin(al) * rot)
    return out

def exp_tidal():
    om = 2 * np.pi / (27.3 * 86400); W = sphere_world(4); land = earth_land(W); gop = gradop(W); beta = earth_beta(W, om, gop)
    ct = Case(W, land, tidal_stress(W, .05), beta, clamp=1e9).run(); cz = Case(W, land, zonal_stress(W, .05), beta, clamp=1e9).run()
    c2 = Case(W, land, tidal_stress(W, .1), beta, clamp=1e9).run()
    hops = coast_hops(W, ct.eff); deep = ct.eff & (hops >= 3) & (hops < 10 ** 9)
    print("[tidal L4] max speed", ct.sp.max(), "unanch", ct.unanch)
    print("   curl nonzonal tidal", nonzonal_fraction(W, deep, ct.curl, 5), "zonal ctl", nonzonal_fraction(W, deep, cz.curl, 5))
    print("   psi nonzonal tidal", nonzonal_fraction(W, ct.eff, ct.psi, 5), "zonal ctl", nonzonal_fraction(W, ct.eff, cz.psi, 5))
    print("   vorticity balance", vorticity_balance(W, ct.op, ct.eff, ct.u, beta, ct.curl, DEF['r'], RHO, DEF['H'], 3))
    print("   doubling err", abs(c2.psi - 2 * ct.psi).max() / abs(ct.psi).max())
    ce = Case(W, land, zonal_stress(W, .1), earth_beta(W)).run()
    print("   (earth L4 for comparison) vorticity balance", vorticity_balance(W, ce.op, ce.eff, ce.u, ce.beta, ce.curl, DEF['r'], RHO, DEF['H'], 3))

if __name__ == "__main__":
    what = sys.argv[1] if len(sys.argv) > 1 else "all"
    if what in ("sphere", "all"): exp_sphere()
    if what in ("water", "all"): exp_waterworld()
    if what in ("plane", "all"): exp_plane()
    if what in ("cube", "all"): exp_cube()
    if what in ("tidal", "all"): exp_tidal()
