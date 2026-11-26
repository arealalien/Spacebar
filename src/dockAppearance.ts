import { useEffect, useState } from 'react';
import type { Settings } from './model';
import { sampleSpotifyArtwork } from './spotifyArtworkColors';
import { spotify } from './spotifyPlayback';

const spotifyGreen = '#1db954';

export function useDockAppearance(settings: Settings, visible = true) {
  const reactive = settings.material === 'spotify';
  const artwork = spotify.useValue(reactive && visible)?.artwork;
  const [sample, setSample] = useState<string>();

  useEffect(() => {
    if (!artwork) return;
    let active = true;
    const image = new Image();
    const publish = (tint: string) => { if (active) setSample(tint); };
    image.onload = () => {
      publish(sampleSpotifyArtwork(image)?.bright ?? spotifyGreen);
    };
    image.onerror = () => publish(spotifyGreen);
    image.src = artwork;
    return () => { active = false; image.onload = image.onerror = null; };
  }, [artwork]);

  const tint = reactive ? (artwork && sample ? sample : spotifyGreen) : settings.tintColor;
  return {
    tint,
    material: reactive ? 'tinted' : settings.material,
    'data-spotify-style': reactive ? (settings.spotifyFrosted ? 'frosted' : 'solid') : undefined,
  } as const;
}
