export interface SpotifyArtworkColors {
  dark: string;
  bright: string;
}

export function sampleSpotifyArtwork(image: HTMLImageElement): SpotifyArtworkColors | undefined {
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = 32;
  const context = canvas.getContext('2d');
  if (!context) return;

  let pixels: Uint8ClampedArray;
  try {
    context.drawImage(image, 0, 0, canvas.width, canvas.height);
    pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
  } catch {
    return;
  }

  // Group similar pixels so large color regions outweigh isolated details.
  const swatches = new Map<number, { red: number; green: number; blue: number; count: number }>();
  let largest = 0;
  for (let index = 0; index < pixels.length; index += 4) {
    if (pixels[index + 3] < 128) continue;
    const red = pixels[index];
    const green = pixels[index + 1];
    const blue = pixels[index + 2];
    const key = (red >> 5) * 64 + (green >> 5) * 8 + (blue >> 5);
    const swatch = swatches.get(key) ?? { red: 0, green: 0, blue: 0, count: 0 };
    swatch.red += red;
    swatch.green += green;
    swatch.blue += blue;
    swatch.count++;
    largest = Math.max(largest, swatch.count);
    swatches.set(key, swatch);
  }
  if (!largest) return;

  let dark = { color: '', score: -1 };
  let bright = dark;
  for (const { red, green, blue, count } of swatches.values()) {
    if (count < Math.min(4, largest)) continue;
    const highest = Math.max(red, green, blue) / count / 255;
    const lowest = Math.min(red, green, blue) / count / 255;
    const lightness = (highest + lowest) / 2;
    const chroma = highest - lowest;
    const coverage = Math.sqrt(count);
    const darkScore = coverage * (1 - lightness) ** 2;
    const brightScore = coverage * (.2 + 5 * chroma) * (.25 + lightness);
    const color = `rgb(${Math.round(red / count)} ${Math.round(green / count)} ${Math.round(blue / count)})`;
    if (darkScore > dark.score) dark = { color, score: darkScore };
    if (brightScore > bright.score) bright = { color, score: brightScore };
  }
  return { dark: dark.color, bright: bright.color };
}
