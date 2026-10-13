/**
 * Colour maths for the tests of the design tokens: `oklch()` as styles.css writes colours, blending
 * a translucent colour over a surface the way a browser does, and WCAG contrast.
 */

/** Red, green and blue as linear light. A channel outside 0..1 means the colour is outside the sRGB gamut. */
export type Rgb = readonly [number, number, number];

const OKLCH = /^oklch\(\s*(\d*\.?\d+)\s+(\d*\.?\d+)\s+(\d*\.?\d+)\s*\)$/;

/** `oklch(L C h)`, hue in degrees, as linear-light sRGB. */
export function oklch(text: string): Rgb {
  const match = OKLCH.exec(text.trim());
  if (match === null) throw new Error(`not an oklch colour: ${text}`);
  const lightness = Number(match[1]);
  const hue = (Number(match[3]) * Math.PI) / 180;
  const a = Number(match[2]) * Math.cos(hue);
  const b = Number(match[2]) * Math.sin(hue);
  const l = (lightness + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (lightness - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (lightness - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
}

const clamp = (value: number) => Math.min(1, Math.max(0, value));
/** Linear light to the gamma-encoded value of sRGB, 0..1. */
const encode = (linear: number) => {
  const c = clamp(linear);
  return c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055;
};
/** The gamma-encoded value of sRGB, 0..1, to linear light. */
const decode = (value: number) => (value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4);

/** The 8-bit red, green and blue a display shows for the colour (clipped to the sRGB gamut). */
export function bytes(color: Rgb): [number, number, number] {
  return [Math.round(encode(color[0]) * 255), Math.round(encode(color[1]) * 255), Math.round(encode(color[2]) * 255)];
}

/** Whether the colour can be shown by an sRGB display as it is, without clipping. */
export function inGamut(color: Rgb): boolean {
  return color.every((channel) => channel >= -0.0005 && channel <= 1.0005);
}

/**
 * `alpha` of `over` laid on `under`. A browser composites in gamma-encoded sRGB (`"srgb"`, which
 * Chrome's pixels confirm); `"linear"` blends light itself, which gives a lighter tint over a dark
 * surface and so lower contrast for light text, and is the stricter reading there.
 */
export function blend(over: Rgb, under: Rgb, alpha: number, space: "srgb" | "linear" = "srgb"): Rgb {
  if (space === "linear") return [0, 1, 2].map((i) => alpha * clamp(over[i]) + (1 - alpha) * clamp(under[i])) as unknown as Rgb;
  return [0, 1, 2].map((i) => decode(alpha * encode(over[i]) + (1 - alpha) * encode(under[i]))) as unknown as Rgb;
}

const luminance = ([r, g, b]: Rgb) => 0.2126 * clamp(r) + 0.7152 * clamp(g) + 0.0722 * clamp(b);

/** The WCAG contrast ratio of two colours, 1 to 21. */
export function contrast(a: Rgb, b: Rgb): number {
  const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (light + 0.05) / (dark + 0.05);
}
