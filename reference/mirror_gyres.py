"""Numpy mirror of Part 4a-2c (ocean_currents, beta off): same geometry as generate_sphere, same CurlOp / GradientOp /
OceanGraph math as the Rust code. Used in place of running the Rust tests (no toolchain in the session);
the thresholds in tests/part4a2c.rs come from here.   python mirror_gyres.py"""
import numpy as np, scipy.sparse as sp, scipy.sparse.linalg as sla
from mir import sphere, frames, rotm, gradop

R = 6.371e6

def curlop3(C, up, north, east, area, adj, el, ed):
    out = []
    for i in range(len(C)):
        S = sum(l * d for l, d in zip(el[i], ed[i]))
        ds = []; cs = []; ws = []
        for j, l, d in zip(adj[i], el[i], ed[i]):
            v = C[j] - C[i]; v = v - up[i] * (v @ up[i]); n = v / np.linalg.norm(v)
            dv = n * d; t = np.cross(up[i], n)
            ds.append([dv @ east[i], dv @ north[i]]); cs.append([2 * l / S * (t @ east[i]), 2 * l / S * (t @ north[i])]); ws.append(1 / d ** 2)
        ds = np.array(ds); cs = np.array(cs); ws = np.array(ws)
        B0 = ds.T @ cs
        S2 = (ds * ws[:, None]).T @ ds
        S2 = S2 + 1e-10 * np.trace(S2) * np.eye(2)
        T = np.array([[0, 1.], [-1, 0]])
        X = ((T - B0).T) @ np.linalg.inv(S2)
        cs = cs + ws[:, None] * (ds @ X.T)
        out.append([east[i] * c[0] + north[i] * c[1] for c in cs])
    return out

def build(L, rot=None):
    C, up, area, adj, el, ed = sphere(L, R, rot)
    north, east = frames(up)
    return dict(C=C, up=up, area=area, adj=adj, el=el, ed=ed, north=north, east=east, n=len(C))

def curl_masked(W, op, v, ocean):
    out = np.zeros(W['n'])
    for i in range(W['n']):
        if not ocean[i]: continue
        s = 0.0
        for c, j in zip(op[i], W['adj'][i]):
            if ocean[j]: s += c @ (v[j] - v[i])
        out[i] = s
    return out

def solve_psi(W, ocean, b):
    """(-L) psi = b on ocean cells, psi = 0 on land (Dirichlet), mean-zero when no coast anchor. Returns global psi."""
    n = W['n']; loc = -np.ones(n, int); cells = np.where(ocean)[0]; loc[cells] = np.arange(len(cells))
    rows = []; cols = []; vals = []
    for l, i in enumerate(cells):
        diag = 0.0
        for j, e, d in zip(W['adj'][i], W['el'][i], W['ed'][i]):
            c = e / d
            diag += c
            if ocean[j]: rows.append(l); cols.append(loc[j]); vals.append(-c)
        rows.append(l); cols.append(l); vals.append(diag)
    K = sp.csr_matrix((vals, (rows, cols)), shape=(len(cells), len(cells)))
    A = W['area'][cells]
    rhs = A * b[cells]
    x, info = sla.cg(K, rhs, rtol=1e-12, atol=0, maxiter=20000, M=sp.diags(1 / K.diagonal()))
    assert info == 0
    psi = np.zeros(n); psi[cells] = x
    return psi

def current(W, gop, psi, ocean):
    n = W['n']; u = np.zeros((n, 3))
    for i in range(n):
        if not ocean[i]: continue
        g = np.zeros(3)
        for c, j in zip(gop[i], W['adj'][i]): g += c * (psi[j] - psi[i])
        g -= W['up'][i] * (g @ W['up'][i])
        u[i] = np.cross(W['up'][i], g)
    return u

def lat_of(W): return np.arcsin(np.clip(W['up'][:, 2], -1, 1))

def world(L, rot=None):
    """Two meridional land strips (lon 0 and 180, |lat| < 70) -> one ocean joined over the poles."""
    W = build(L, rot)
    C = W['C']; lat = lat_of(W)
    land = (np.abs(C[:, 1]) < 0.13 * R) & (np.abs(lat) < np.radians(70))
    return W, ~land

def synthetic_stress(W, tau0=0.1):
    lat = lat_of(W)
    return (-tau0 * np.cos(3 * lat))[:, None] * W['east']      # trades at the equator, westerlies at 60 deg

def run(L, H=100., r=5e-8, rho=1025., vmax=3.):
    W, ocean = world(L)
    op = curlop3(W['C'], W['up'], W['north'], W['east'], W['area'], W['adj'], W['el'], W['ed'])
    gop = gradop(W['C'], W['up'], W['north'], W['east'], W['adj'], W['ed'])
    tau = synthetic_stress(W)
    cu = curl_masked(W, op, tau, ocean)
    b = -cu / (rho * H * r)            # (-L) psi = -curl/(rho H r)  [standard psi, u = k x grad psi]
    psi = solve_psi(W, ocean, b)
    u = current(W, gop, psi, ocean)
    return W, ocean, op, gop, tau, cu, psi, u

if __name__ == "__main__":
    for L in [3, 4]:
        W, ocean, op, gop, tau, cu, psi, u = run(L)
        n = W['n']; lat = np.degrees(lat_of(W)); sp_ = np.linalg.norm(u, axis=1)
        print("L", L, "ocean", ocean.sum(), "/", n, "max|psi|", np.abs(psi).max(), "max speed", sp_.max(), "clamped(>3)", (sp_ > 3).sum())
        # vorticity of the current (curl of u), compared with curl(tau)/(rho H r)
        z = curl_masked(W, op, u, ocean)
        for lo, hi in [(10, 40), (-40, -10)]:
            m = ocean & (lat > lo) & (lat < hi) & (np.abs(W['C'][:, 1]) > 0.5 * R)
            print(f"  lat {lo}..{hi}: mean vorticity {z[m].mean():.3e}  mean curl_tau {cu[m].mean():.3e}  mean psi {psi[m].mean():.3e}")
        # coast normal flux
        worst = 0.0; worst_rel = 0.0
        for i in np.where(ocean)[0]:
            for j in W['adj'][i]:
                if not ocean[j]:
                    v = W['C'][j] - W['C'][i]; v -= W['up'][i] * (v @ W['up'][i]); v /= np.linalg.norm(v)
                    f = abs(u[i] @ v); worst = max(worst, f)
        print("  coast normal speed max", worst, " / max speed", worst / sp_.max())
