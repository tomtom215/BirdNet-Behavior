// Colour maths shared by the token gates (chart-ramps.mjs): OKLCH <-> sRGB, WCAG contrast, OKLab dE, Machado 2009 CVD (severity 1.0).
export const parse = (s) => { // "oklch(56% 0.12 150)" or with "/ a"
  const m = s.match(/oklch\(\s*([\d.]+)%\s+([\d.]+)\s+([\d.]+)(?:\s*\/\s*([\d.]+))?\s*\)/);
  if (!m) throw new Error('not oklch: ' + s);
  return { L: +m[1] / 100, C: +m[2], H: +m[3], A: m[4] === undefined ? 1 : +m[4] };
};
export const fmt = ({ L, C, H }) => `oklch(${+(L * 100).toFixed(1)}% ${+C.toFixed(3)} ${+H.toFixed(0)})`;
export function oklchToLinear({ L, C, H }) {
  const a = C * Math.cos((H * Math.PI) / 180), b = C * Math.sin((H * Math.PI) / 180);
  const l_ = L + 0.3963377774 * a + 0.2158037573 * b, m_ = L - 0.1055613458 * a - 0.0638541728 * b, s_ = L - 0.0894841775 * a - 1.2914855480 * b;
  const l = l_ ** 3, m = m_ ** 3, s = s_ ** 3;
  return [4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s, -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s, -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s];
}
export const inGamut = (c) => oklchToLinear(c).every((v) => v >= -0.0005 && v <= 1.0005);
export const gamutMap = (c) => { // reduce chroma until in sRGB
  let x = { ...c };
  while (!inGamut(x) && x.C > 0) x = { ...x, C: x.C - 0.002 };
  return x;
};
const enc = (v) => { v = Math.min(1, Math.max(0, v)); return v <= 0.0031308 ? 12.92 * v : 1.055 * v ** (1 / 2.4) - 0.055; };
export const toHex = (c) => '#' + oklchToLinear(gamutMap(c)).map((v) => Math.round(enc(v) * 255).toString(16).padStart(2, '0')).join('');
export const lum = (c) => { const [r, g, b] = oklchToLinear(gamutMap(c)).map((v) => Math.min(1, Math.max(0, v))); return 0.2126 * r + 0.7152 * g + 0.0722 * b; };
export const contrast = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
export const lab = ({ L, C, H }) => [L, C * Math.cos((H * Math.PI) / 180), C * Math.sin((H * Math.PI) / 180)];
export const dE = (a, b) => { const [p, q] = [lab(a), lab(b)]; return 100 * Math.hypot(p[0] - q[0], p[1] - q[1], p[2] - q[2]); };
// Machado 2009 severity 1.0, applied to linear sRGB.
const M = { protan: [[0.152286, 1.052583, -0.204868], [0.114503, 0.786281, 0.099216], [-0.003882, -0.048116, 1.051998]], deutan: [[0.367322, 0.860646, -0.227968], [0.280085, 0.672501, 0.047413], [-0.011820, 0.042940, 0.968881]] };
const linToOklab = ([r, g, b]) => { const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b), m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b), s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b); return [0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s, 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s, 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s]; };
export function dECvd(a, b, kind) {
  const f = (c) => { const v = oklchToLinear(gamutMap(c)).map((x) => Math.min(1, Math.max(0, x))); const m = M[kind]; return linToOklab(m.map((r) => r[0] * v[0] + r[1] * v[1] + r[2] * v[2])); };
  const [p, q] = [f(a), f(b)]; return 100 * Math.hypot(p[0] - q[0], p[1] - q[1], p[2] - q[2]);
}

// ── compositing, for a translucent colour over an opaque one ──────────────
const gammaEnc = (v) => { v = Math.min(1, Math.max(0, v)); return v <= 0.0031308 ? 12.92 * v : 1.055 * v ** (1 / 2.4) - 0.055; };
const gammaDec = (v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
/** sRGB, gamma-encoded, 0..1, gamut-mapped. */
export const toSrgb = (c) => oklchToLinear(gamutMap(c)).map(gammaEnc);
/** `top` (with its own alpha, c.A) over opaque `under`, composited in encoded sRGB as a browser does. */
export function over(top, under) {
  const a = top.A ?? 1;
  const t = toSrgb(top), u = toSrgb(under);
  return t.map((v, i) => a * v + (1 - a) * u[i]);
}
const lumSrgb = (rgb) => { const [r, g, b] = rgb.map(gammaDec); return 0.2126 * r + 0.7152 * g + 0.0722 * b; };
/** WCAG contrast between two encoded-sRGB triples. */
export function contrastRgb(a, b) { const [x, y] = [lumSrgb(a), lumSrgb(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); }
