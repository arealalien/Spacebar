import { useEffect, useRef } from 'react';

export function ColorPicker({ id, label, value, onInput, onCommit }: {
  id?: string;
  label: string;
  value: string;
  onInput: (value: string) => void;
  onCommit?: (value: string) => void;
}) {
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const element = input.current;
    if (!element || !onCommit) return;
    const commit = () => onCommit(element.value);
    element.addEventListener('change', commit);
    return () => element.removeEventListener('change', commit);
  }, [onCommit]);

  return <input ref={input} id={id} className="color-picker" type="color" aria-label={label}
    value={value} onInput={(event) => onInput(event.currentTarget.value)} />;
}
