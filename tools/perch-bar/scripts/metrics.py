"""Measurement library (Task 3).  No I/O besides what the callers pass in.

A *window table* is computed once per set of windows from the raw Perch logits:
  P (softmax share, float32), top-K classes per window, the pair table (present species x window), FSD total, ...
Bars are applied on softmax SHARES unless a function says otherwise.
"""
import numpy as np

BARS = [0.05, 0.10, 0.15, 0.20, 0.25, 0.30, 0.35, 0.40, 0.50, 0.70]
K = 40


def softmax32(Z):
    Z = Z.astype(np.float64)
    Z = Z - Z.max(axis=1, keepdims=True)
    E = np.exp(Z)
    return (E / E.sum(axis=1, keepdims=True)).astype(np.float32)


def parse_ids(s):
    return [int(x) for x in s.split("|")] if s else []


def window_table(Z, present, edge, is_species, region_mask=None):
    """Z: (n,C) logits; present/edge: list of lists of class indices (ground truth per window);
    is_species: (C,) bool (False = the 200 FSD50K sound classes); region_mask: optional 'oracle local list' = one (C,) bool
    array for all windows, or a list with one such array per window.
    Returns dict of arrays."""
    n, C = Z.shape
    P = softmax32(Z)
    ti = np.argpartition(-P, K, axis=1)[:, :K]
    ts = np.take_along_axis(P, ti, 1)
    o = np.argsort(-ts, axis=1)
    top_idx = np.take_along_axis(ti, o, 1)
    top_sh = np.take_along_axis(ts, o, 1)
    fsd_total = P[:, ~is_species].sum(axis=1)
    # pair table
    pw, pc, psh, plg, prk = [], [], [], [], []
    for w in range(n):
        for ci in present[w]:
            pw.append(w); pc.append(ci); psh.append(P[w, ci]); plg.append(Z[w, ci])
            prk.append(int((P[w] > P[w, ci]).sum()) + 1)
    # largest absent share (present excluded; and: present+edge excluded = 'strict')
    mx_abs = np.zeros(n, np.float32); mx_abs_strict = np.zeros(n, np.float32)
    mx_abs_sp_strict = np.zeros(n, np.float32)
    fr_sp = np.zeros((n, len(BARS)), np.int32); fr_fsd = np.zeros((n, len(BARS)), np.int32)
    fr_reg = np.zeros((n, len(BARS)), np.int32)
    any_ge = np.zeros((n, len(BARS)), bool); any_sp_ge = np.zeros((n, len(BARS)), bool)
    for w in range(n):
        pset, eset = set(present[w]), set(edge[w])
        rmask = None if region_mask is None else (region_mask[w] if isinstance(region_mask, list) else region_mask)
        a = s = ss = 0.0
        for ci, sh in zip(top_idx[w], top_sh[w]):
            ci = int(ci)
            if ci in pset: continue
            if sh > a: a = float(sh)
            if ci in eset: continue
            if sh > s: s = float(sh)
            if is_species[ci] and sh > ss: ss = float(sh)
            for j, t in enumerate(BARS):
                if sh >= t:
                    if is_species[ci]:
                        fr_sp[w, j] += 1
                        if rmask is not None and rmask[ci]: fr_reg[w, j] += 1
                    else: fr_fsd[w, j] += 1
        mx_abs[w], mx_abs_strict[w], mx_abs_sp_strict[w] = a, s, ss
        t1 = top_sh[w, 0]
        for j, t in enumerate(BARS):
            any_ge[w, j] = t1 >= t
        tsp = next((sh for ci, sh in zip(top_idx[w], top_sh[w]) if is_species[int(ci)]), 0.0)
        for j, t in enumerate(BARS):
            any_sp_ge[w, j] = tsp >= t
    return dict(P=P, top_idx=top_idx, top_sh=top_sh, fsd_total=fsd_total,
                pair_win=np.array(pw, int), pair_cls=np.array(pc, int), pair_share=np.array(psh, np.float32),
                pair_logit=np.array(plg, np.float32), pair_rank=np.array(prk, int),
                max_absent=mx_abs, max_absent_strict=mx_abs_strict, max_absent_sp_strict=mx_abs_sp_strict,
                fr_sp=fr_sp, fr_fsd=fr_fsd, fr_reg=fr_reg, any_ge=any_ge, any_sp_ge=any_sp_ge)


def wilson(k, n, z=1.959964):
    if n == 0: return (float("nan"),) * 3
    p = k / n
    den = 1 + z * z / n
    cen = (p + z * z / (2 * n)) / den
    half = z * np.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / den
    return p, max(0.0, cen - half), min(1.0, cen + half)


def cluster_ratio_ci(num, den, cl, B=2000, seed=1):
    """Point estimate sum(num)/sum(den) with a percentile CI from resampling whole clusters (cluster = recording / segment).
    num, den: per-item arrays; cl: per-item cluster labels."""
    num = np.asarray(num, float); den = np.asarray(den, float)
    if den.sum() == 0: return (float("nan"),) * 3
    u, inv = np.unique(np.asarray(cl), return_inverse=True)
    cn = np.bincount(inv, weights=num, minlength=len(u)); cd = np.bincount(inv, weights=den, minlength=len(u))
    k = len(u)
    if k < 2: return (cn.sum() / cd.sum(), float("nan"), float("nan"))
    rng = np.random.default_rng(seed)
    W = rng.multinomial(k, np.full(k, 1.0 / k), size=B).astype(float)
    dd = W @ cd
    ok = dd > 0
    r = (W @ cn)[ok] / dd[ok]
    return (cn.sum() / cd.sum(), float(np.percentile(r, 2.5)), float(np.percentile(r, 97.5)))


def fmt_ci(p, lo, hi, pct=True, nd=1):
    if p != p: return "n/a"
    if pct: return f"{100*p:.{nd}f} [{100*lo:.{nd}f}-{100*hi:.{nd}f}]"
    return f"{p:.{nd+1}f} [{lo:.{nd+1}f}-{hi:.{nd+1}f}]"


def prob_table(Pb, present, edge):
    """Same bookkeeping as window_table for a model whose outputs are already probabilities (BirdNET+).
    Returns fr[n,len(BARS_B)], any_ge[n,len(BARS_B)], pair table (win, cls, prob, rank)."""
    n, C = Pb.shape
    ti = np.argpartition(-Pb, K, axis=1)[:, :K]
    ts = np.take_along_axis(Pb, ti, 1)
    o = np.argsort(-ts, axis=1)
    top_idx = np.take_along_axis(ti, o, 1); top_sh = np.take_along_axis(ts, o, 1)
    fr = np.zeros((n, len(BARS_B)), np.int32); any_ge = np.zeros((n, len(BARS_B)), bool)
    pw, pc, pp, prk = [], [], [], []
    for w in range(n):
        pset, eset = set(present[w]), set(edge[w])
        for ci in present[w]:
            pw.append(w); pc.append(ci); pp.append(Pb[w, ci]); prk.append(int((Pb[w] > Pb[w, ci]).sum()) + 1)
        for ci, sh in zip(top_idx[w], top_sh[w]):
            ci = int(ci)
            if ci in pset or ci in eset: continue
            for j, t in enumerate(BARS_B):
                if sh >= t: fr[w, j] += 1
        for j, t in enumerate(BARS_B):
            any_ge[w, j] = top_sh[w, 0] >= t
    return dict(top_idx=top_idx, top_sh=top_sh, fr=fr, any_ge=any_ge, pair_win=np.array(pw, int), pair_cls=np.array(pc, int),
                pair_prob=np.array(pp, np.float32), pair_rank=np.array(prk, int))

BARS_B = [0.05, 0.10, 0.15, 0.20, 0.25, 0.30, 0.35, 0.40, 0.50, 0.70, 0.80, 0.90]
