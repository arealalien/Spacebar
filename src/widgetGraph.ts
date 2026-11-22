export function smoothLine(samples: readonly number[], start = 2, end = 98) {
  if (samples.length < 2) return '';
  const step = (end - start) / (samples.length - 1);
  const slopes = samples.slice(1).map((sample, index) => sample - samples[index]);
  const tangents = samples.map((_, index) => {
    const before = slopes[index - 1] ?? slopes[0];
    const after = slopes[index] ?? slopes[slopes.length - 1];
    // At turning points, flatten the tangent so the curve cannot invent a peak.
    return before * after > 0 ? 2 * before * after / (before + after) : 0;
  });
  const point = (x: number, y: number) => x.toFixed(2) + ' ' + y.toFixed(2);
  let path = 'M' + point(start, samples[0]);
  for (let index = 0; index < slopes.length; index++) {
    const x = start + index * step;
    path += ' C' + point(x + step / 3, samples[index] + tangents[index] / 3)
      + ' ' + point(x + 2 * step / 3, samples[index + 1] - tangents[index + 1] / 3)
      + ' ' + point(x + step, samples[index + 1]);
  }
  return path;
}
