"""Part 4a-4 numpy mirror: plane coastal upwelling (sign and magnitude), salinity diffusion (mean, bounds), sea-ice freezing point.
Same formulas as src/ocean_secondary.rs on a walled plane nx x ny with 10 km cells. Run: python3 reference/mirror_4a4.py"""
import numpy as np

nx, ny, dx = 24, 16, 1.0e4
land = np.zeros((ny, nx), bool); land[:, :6] = True; land[:, 18:] = True
ocean = ~land
east, north, up = np.array([1., 0, 0]), np.array([0., 1, 0]), np.array([0., 0, 1])

def coastal(f, tau0, rho=1025.0):
    w = np.zeros((ny, nx))
    for j in range(ny):
        for i in range(nx):
            if not ocean[j, i]: continue
            S = np.zeros(3)
            for di, dj, n in [(1, 0, east), (-1, 0, -east), (0, 1, north), (0, -1, -north)]:
                ii, jj = i + di, j + dj
                if 0 <= ii < nx and 0 <= jj < ny and not ocean[jj, ii]:
                    S += dx * n
            if not S.any(): continue
            v = tau0 * north / (rho * f)
            M = np.cross(v, up)
            w[j, i] = -np.dot(M, S) / dx**2
    return w

for f in (1e-4, -1e-4):
    w = coastal(f, 0.1)
    want = 0.1 / (1025 * 1e-4) * dx / dx**2
    print(f"f={f:+.0e}: west coast {w[3, 6]:+.4e}, east coast {w[3, 17]:+.4e}, expected |w| {want:.4e}, interior {w[3, 11]}")
    assert abs(w[3, 6] - np.sign(f) * want) < 1e-3 * want and abs(w[3, 17] + np.sign(f) * want) < 1e-3 * want

# Salinity: clamp, remove area mean over ocean, conservative diffusion on the ocean graph
rng = np.random.default_rng(1)
sig = np.clip(rng.normal(0, 4, (ny, nx)), -5, 5) * ocean
sal = np.where(ocean, 35 + sig - sig[ocean].mean(), 0.0)
theta = 0.25 * 1.0 / 4  # A/(sum w) with w = 1 per edge (dx/dx), A = dx^2 -> scale-free form: theta = 0.25 * A / ws, flux uses w -> use unit cells
for _ in range(20):
    nxt = sal.copy()
    for j in range(ny):
        for i in range(nx):
            if not ocean[j, i]: continue
            s = 0.0
            for di, dj in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
                ii, jj = i + di, j + dj
                if 0 <= ii < nx and 0 <= jj < ny and ocean[jj, ii]: s += sal[jj, ii] - sal[j, i]
            nxt[j, i] = sal[j, i] + 0.25 * 0.25 * s  # theta*w/A with unit cells = 0.25 * 1 / 4 per step (<= max-principle limit)
    sal = nxt
print("salinity mean", sal[ocean].mean(), "min/max", sal[ocean].min(), sal[ocean].max())
assert abs(sal[ocean].mean() - 35) < 1e-9 and sal[ocean].min() >= 25 and sal[ocean].max() <= 45

# Freezing point
tf = lambda S: 271.35 - 0.054 * (S - 35)
print("T_f(35)=%.3f T_f(30)=%.3f" % (tf(35), tf(30)))
assert tf(30) > 271.5 > tf(35)
print("mirror OK")
