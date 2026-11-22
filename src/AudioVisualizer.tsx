import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import { desktop } from './model';
import { audioDefaults, type AudioVisualizerOptions } from './widgets';
import { smoothLine } from './widgetGraph';

const example = [.04, .12, .42, .18, .29, .66, .31, .19, .48, .85, .52, .31, .52, .26, .12, .06];
const silent: number[] = example.map(() => 0);
const sampleInterval = 32;
const response = {
  slow: { attack: 44, release: 235 },
  normal: { attack: 16, release: 125 },
  fast: { attack: 8, release: 66 },
};
const sensitivityGain = { low: -8, normal: 0, high: 8 };

export function AudioVisualizer({ specimen, active = true, options }: {
  specimen: boolean; active?: boolean; options?: Partial<AudioVisualizerOptions>;
}) {
  const [levels, setLevels] = useState(silent);
  const [available, setAvailable] = useState(true);
  const alignment = options?.audioAlignment ?? audioDefaults.audioAlignment;
  const speed = options?.audioSpeed ?? audioDefaults.audioSpeed;
  const sensitivity = options?.audioSensitivity ?? audioDefaults.audioSensitivity;

  useEffect(() => {
    if (specimen || !active || !desktop) {
      setLevels(silent);
      return;
    }
    const motion = matchMedia('(prefers-reduced-motion: reduce)');
    const { attack, release } = response[speed] ?? response.normal;
    const gain = sensitivityGain[sensitivity] ?? sensitivityGain.normal;
    let timer: number | undefined;
    let reading = false;
    let closed = false;
    let lastFrameAt = 0;

    async function sample() {
      if (closed || document.hidden || reading) return;
      reading = true;
      const started = performance.now();
      const bands = await invoke<number[] | null>('audio_output_spectrum').catch(() => null);
      reading = false;
      if (closed || document.hidden) return;
      const now = performance.now();
      const elapsed = lastFrameAt ? now - lastFrameAt : sampleInterval;
      lastFrameAt = now;
      const valid = Array.isArray(bands) && bands.length === silent.length && bands.every(Number.isFinite);
      setAvailable(valid);
      if (valid) {
        setLevels((previous) => {
          const next = bands.map((amplitude, index) => {
            const decibels = 20 * Math.log10(Math.max(amplitude, 0.000001));
            const target = Math.min(1, Math.max(0, (decibels + 56 + gain) / 46)) ** 1.25;
            const current = previous[index];
            const time = target > current ? attack : release;
            const height = target + (current - target) * Math.exp(-elapsed / time);
            return height < .004 ? 0 : height;
          });
          return next.every((height, index) => height === previous[index]) ? previous : next;
        });
      } else {
        setLevels(silent);
      }
      const cadence = valid ? (motion.matches ? 240 : sampleInterval) : 280;
      timer = window.setTimeout(sample, Math.max(4, cadence - (now - started)));
    }

    function visibility() {
      window.clearTimeout(timer);
      if (document.hidden) {
        lastFrameAt = 0;
        setLevels(silent);
      } else {
        void sample();
      }
    }
    document.addEventListener('visibilitychange', visibility);
    void sample();
    return () => {
      closed = true;
      window.clearTimeout(timer);
      document.removeEventListener('visibilitychange', visibility);
    };
  }, [specimen, active, speed, sensitivity]);

  const values = specimen || !desktop ? example : active ? levels : silent;
  const label = available ? 'Audio output visualizer' : 'Audio output unavailable';
  const centered = alignment === 'center';
  const baseline = centered ? 16 : 29;
  const wave = smoothLine(values.map((level) => baseline - level * (centered ? 14 : 25)));
  const resting = values.every((level) => level === 0);
  return <span className="widget-face audio-visualizer-face" role="img" aria-label={label} title={label}>
    <svg className="audio-spectrum" data-resting={resting} viewBox="0 0 100 32" preserveAspectRatio="none" aria-hidden="true">
      {centered && !resting && <path className="audio-spectrum-reflection" d={wave} transform="translate(0 32) scale(1 -1)" />}
      <path d={wave} />
    </svg>
  </span>;
}
