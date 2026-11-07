"""Calibration runs for tests/part4a2c.rs (coast slip, bands, plane, divergence, ring psi). Uses mirror_gyres.py."""
from mirror_gyres import *

def coast_S(W, ocean):
    S = np.zeros((W['n'], 3)); Ls = np.zeros(W['n'])
    for i in np.where(ocean)[0]:
        for j, e in zip(W['adj'][i], W['el'][i]):
            if not ocean[j]:
                v = W['C'][j] - W['C'][i]; v -= W['up'][i] * (v @ W['up'][i]); v /= np.linalg.norm(v)
                S[i] += e * v; Ls[i] += e
    return S, Ls

def slip(W, ocean, u):
    S, Ls = coast_S(W, ocean); u = u.copy(); cnt = 0
    for i in np.where(ocean)[0]:
        s = np.linalg.norm(S[i])
        if Ls[i] > 0 and s > 1e-6 * Ls[i]:
            sh = S[i] / s; u[i] -= (u[i] @ sh) * sh; cnt += 1
    return u, cnt

def outflux(W, ocean, u):
    S, Ls = coast_S(W, ocean)
    return np.array([u[i] @ S[i] for i in range(W['n'])]), Ls

def deep_mask(W, ocean, hops=2):
    d = ocean.copy()
    for i in range(W['n']):
        if ocean[i] and any((not ocean[j]) or any(not ocean[k] for k in W['adj'][j]) for j in W['adj'][i]): d[i] = False
    return d

def div_ls(W, gop, u, mask):
    out = np.zeros(W['n'])
    for i in np.where(mask)[0]:
        out[i] = sum(c @ (u[j] - u[i]) for c, j in zip(gop[i], W['adj'][i]))
    return out

def plane(nx, ny, h=1e4):
    n = nx * ny; C = np.zeros((n, 3)); adj = []; el = []; ed = []
    for j in range(ny):
        for i in range(nx):
            k = j * nx + i; C[k] = [i * h, j * h, 0]
            nb = sorted(jj * nx + ii for ii, jj in [(i - 1, j), (i + 1, j), (i, j - 1), (i, j + 1)] if 0 <= ii < nx and 0 <= jj < ny)
            adj.append(nb); el.append([h] * len(nb)); ed.append([h] * len(nb))
    up = np.tile([0, 0, 1.], (n, 1)); north = np.tile([0, 1., 0], (n, 1)); east = np.tile([1., 0, 0], (n, 1))
    return dict(C=C, up=up, area=np.full(n, h * h), adj=adj, el=el, ed=ed, north=north, east=east, n=n, nx=nx, ny=ny, h=h)

if __name__ == "__main__":
    H, r, rho = 100., 5e-8, 1025.
    for L in [3, 4, 5]:
        W, ocean, op, gop, tau, cu, psi, u = run(L)
        us, cnt = slip(W, ocean, u)
        F0, Ls = outflux(W, ocean, u); F1, _ = outflux(W, ocean, us)
        m = Ls > 0; um = np.linalg.norm(u, axis=1).max()
        print(f"L{L}: slipped cells {cnt}/{m.sum()}  max|F|/(Ls*umax) raw {np.abs(F0[m]/Ls[m]).max()/um:.3f}  slipped {np.abs(F1[m]/Ls[m]).max()/um:.2e}")
        deep = deep_mask(W, ocean)
        lat = np.degrees(lat_of(W)); z = curl_masked(W, op, us, ocean)
        for lo, hi in [(10, 40), (-40, -10)]:
            mm = deep & (lat > lo) & (lat < hi)
            ex = cu[mm] / (rho * H * r)
            print(f"   band {lo}..{hi}: n={mm.sum()} mean zeta {z[mm].mean():.4e} mean curl/(rho H r) {ex.mean():.4e} ratio {z[mm].mean()/ex.mean():.3f}")
        zn = z[deep & (lat > 10) & (lat < 40)].mean(); zs = z[deep & (lat < -10) & (lat > -40)].mean(); print("   N/S ratio", zn / zs)
        # coast ring psi
        ring = np.array([ocean[i] and any(not ocean[j] for j in W['adj'][i]) for i in range(W['n'])])
        d3 = deep
        print(f"   psi: ring mean|psi| {np.abs(psi[ring]).mean():.3e}  deep mean|psi| {np.abs(psi[d3]).mean():.3e}  ring/deep {np.abs(psi[ring]).mean()/np.abs(psi[d3]).mean():.3f} ring max/ global max {np.abs(psi[ring]).max()/np.abs(psi).max():.3f}")
        d = div_ls(W, gop, us, deep)
        print(f"   LS div / vort  max {np.abs(d[deep]).max()/np.abs(z[deep]).max():.4f}  rms {np.sqrt((d[deep]**2).mean())/np.sqrt((z[deep]**2).mean()):.4f}")
        sp_ = np.linalg.norm(us, axis=1); print("   speeds: max", sp_.max(), "clamp>3:", (sp_ > 3).sum(), "of", ocean.sum(), " clamp>0.5:", (sp_ > 0.5).sum())
    # plane
    nx, ny = 40, 30
    W = plane(nx, ny); op = curlop3(W['C'], W['up'], W['north'], W['east'], W['area'], W['adj'], W['el'], W['ed'])
    gop = gradop(W['C'], W['up'], W['north'], W['east'], W['adj'], W['ed'])
    ocean = np.array([0 < (k % nx) < nx - 1 and 0 < (k // nx) < ny - 1 for k in range(W['n'])])
    y = W['C'][:, 1]; tau = np.zeros((W['n'], 3)); tau[:, 0] = -0.1 * np.cos(2 * np.pi * y / (ny * W['h']))
    cu = curl_masked(W, op, tau, ocean); b = -cu / (rho * H * r)
    psi = solve_psi(W, ocean, b); u = current(W, gop, psi, ocean)
    us, cnt = slip(W, ocean, u)
    deep = deep_mask(W, ocean); d = div_ls(W, gop, us, deep)
    print("plane: max|u|", np.linalg.norm(u, axis=1).max(), "max|div|*h/max|u| deep", np.abs(d[deep]).max() * W['h'] / np.linalg.norm(u, axis=1).max())
    F1, Ls = outflux(W, ocean, us); F0, _ = outflux(W, ocean, u); m = Ls > 0
    print("plane outflux raw max", np.abs(F0[m]/Ls[m]).max(), "slipped", np.abs(F1[m]/Ls[m]).max())
    # uniform wind
    tau = np.tile([5e-2, 3e-2, 0.], (W['n'], 1)); cu = curl_masked(W, op, tau, ocean); print("uniform curl max", np.abs(cu).max())
