import { invoke } from '@tauri-apps/api/core';
import { useState } from 'react';

export interface InstalledReleaseNotes {
  version: string;
  notes: string;
  url: string;
}

export default function WhatsNewPage({ release }: { release: InstalledReleaseNotes }) {
  const [error, setError] = useState('');
  const changes = release.notes.split(/\r?\n/)
    .map((line) => line.trim().match(/^[-*+]\s+(.+)$/)?.[1])
    .filter((line): line is string => !!line);

  if (changes.length) {
    return <ul className="release-notes release-notes-list">
      {changes.map((change, index) => <li key={index}>{change}</li>)}
    </ul>;
  }

  return <div className="release-notes">
    <a href={release.url} onClick={(event) => {
      event.preventDefault();
      setError('');
      void invoke('open_release_notes_link', { url: release.url })
        .catch(() => setError('Couldn’t open the link. Try again.'));
    }}>View release on GitHub</a>
    {error && <p className="release-notes-error" role="alert">{error}</p>}
  </div>;
}
