import type { PlaybackState } from './PlaybackControls';
import { widgetSnapshot } from './widgetSnapshots';

export interface SpotifyTimeline {
  position: number;
  duration: number;
  updatedAt: number | null;
  playbackRate: number;
  canSeek: boolean;
}

export interface SpotifyTrack extends PlaybackState {
  available: boolean;
  title: string;
  artist: string;
  artwork?: string;
  timeline?: SpotifyTimeline | null;
}

export const spotify = widgetSnapshot<SpotifyTrack>('spotify_snapshot', 1100, 'spotify-changed');
