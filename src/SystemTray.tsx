import { invoke } from '@tauri-apps/api/core';
import { AppWindow, Check, Ellipsis, FolderOpen, LogOut, Settings2, Usb, X } from 'lucide-react';
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type MouseEvent } from 'react';

type TrayIcon = { id: string; label: string; icon: string | null; isSpacebar: boolean; canReveal: boolean };
type UsbDrive = { id: string; label: string; letters: string[] };
type MenuTarget = { app: TrayIcon; name: string } | { name: 'USB drives'; app?: undefined };
type Menu = MenuTarget & { point?: { x: number; y: number } };

export function SystemTray({ onClose, onError }: { onClose: () => void; onError: (error: unknown) => void }) {
  const [icons, setIcons] = useState<TrayIcon[]>();
  const [failure, setFailure] = useState('');
  const [menu, setMenu] = useState<Menu>();
  const [drives, setDrives] = useState<UsbDrive[]>();
  const [menuError, setMenuError] = useState('');
  const [message, setMessage] = useState('');
  const [busy, setBusy] = useState(false);
  const activeMenu = useRef<Menu>(undefined);
  const menuElement = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement | null>(null);
  const running = useRef(false);

  useEffect(() => {
    let alive = true;
    let timer: number;
    async function update() {
      try {
        const entries = await invoke<TrayIcon[]>('read_tray_icons');
        if (alive) { setIcons(entries); setFailure(''); }
      } catch (error) { if (alive) setFailure(String(error)); }
      if (alive) timer = window.setTimeout(update, 2500);
    }
    void update();
    return () => { alive = false; window.clearTimeout(timer); };
  }, []);

  useEffect(() => {
    if (!menu || menu.app) return;
    let alive = true;
    void invoke<UsbDrive[]>('read_tray_usb_drives').then((drives) => {
      if (alive) setDrives(drives);
    }).catch((error) => {
      if (alive) { setDrives([]); setMenuError(String(error)); }
    });
    return () => { alive = false; };
  }, [menu]);

  const dismissMenu = useCallback((restoreFocus = true) => {
    activeMenu.current = undefined;
    setMenu(undefined);
    if (restoreFocus) trigger.current?.focus();
  }, []);

  useEffect(() => {
    if (!menu) return;
    function dismissOutside(event: PointerEvent) {
      const target = event.target as Node;
      if (menuElement.current?.contains(target) || trigger.current?.contains(target)) return;
      dismissMenu(false);
    }
    document.addEventListener('pointerdown', dismissOutside);
    return () => document.removeEventListener('pointerdown', dismissOutside);
  }, [menu, dismissMenu]);

  useLayoutEffect(() => {
    const element = menuElement.current;
    const anchor = trigger.current;
    if (!element || !anchor || !menu) return;
    const margin = 10;
    const position = () => {
      const { offsetWidth: width, offsetHeight: height } = element;
      const bounds = anchor.getBoundingClientRect();
      const left = menu.point?.x ?? bounds.right - width;
      const below = menu.point?.y ?? bounds.bottom + 4;
      const above = menu.point?.y ?? bounds.top - 4;
      const top = below + height + margin <= window.innerHeight ? below : above - height;
      element.style.left = `${Math.max(margin, Math.min(left, window.innerWidth - width - margin))}px`;
      element.style.top = `${Math.max(margin, Math.min(top, window.innerHeight - height - margin))}px`;
    };
    position();
    (element.querySelector<HTMLButtonElement>('button:enabled') ?? element).focus();
    const observer = new ResizeObserver(position);
    observer.observe(element);
    window.addEventListener('resize', position);
    return () => { observer.disconnect(); window.removeEventListener('resize', position); };
  }, [menu]);

  useLayoutEffect(() => {
    const element = menuElement.current;
    if (drives?.length && document.activeElement === element) element?.querySelector<HTMLButtonElement>('button:enabled')?.focus();
  }, [drives]);

  function openMenu(target: MenuTarget, anchor: HTMLButtonElement, point?: { x: number; y: number }) {
    if (!point && activeMenu.current && trigger.current === anchor) {
      dismissMenu();
      return;
    }
    const next = { ...target, point };
    activeMenu.current = next;
    trigger.current = anchor;
    setMenu(next);
    setDrives(undefined);
    setMenuError('');
    setMessage('');
    onError('');
  }

  function contextMenu(event: MouseEvent<HTMLElement>, target: MenuTarget) {
    event.preventDefault();
    const anchor = event.currentTarget.querySelector<HTMLButtonElement>('button') ?? event.currentTarget as HTMLButtonElement;
    openMenu(target, anchor, { x: event.clientX, y: event.clientY });
  }

  function menuShortcut(event: KeyboardEvent<HTMLElement>, target: MenuTarget) {
    if (event.key !== 'ContextMenu' && !(event.shiftKey && event.key === 'F10')) return;
    const anchor = (event.target as HTMLElement).closest('button');
    if (!anchor) return;
    event.preventDefault();
    openMenu(target, anchor);
  }

  function moveFocus(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === 'Escape' || event.key === 'Tab') {
      if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); }
      dismissMenu();
      return;
    }
    const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('button:enabled')];
    if (!buttons.length) return;
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === 'ArrowDown' ? (index + 1) % buttons.length
      : event.key === 'ArrowUp' ? (index <= 0 ? buttons.length - 1 : index - 1)
      : event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : -1;
    if (next >= 0) { event.preventDefault(); buttons[next].focus(); }
  }

  async function run(action: string, id?: string, ejected?: UsbDrive) {
    if (running.current) return;
    const currentMenu = activeMenu.current;
    running.current = true;
    setBusy(true);
    setMenuError('');
    setMessage('');
    try {
      await invoke(action, id === undefined ? {} : { id });
      if (activeMenu.current !== currentMenu) return;
      if (ejected) {
        setDrives((current) => current?.filter((drive) => drive.id !== ejected.id));
        setMessage(`You can unplug ${ejected.label}.`);
        menuElement.current?.focus();
      } else dismissMenu();
    } catch (error) {
      if (activeMenu.current === currentMenu) { setMenuError(String(error)); menuElement.current?.focus(); }
    } finally {
      running.current = false;
      setBusy(false);
    }
  }

  return <>
    <header className="quick-header">
      <h1 id="system-panel-title">System tray</h1>
      <button type="button" className="quick-round" aria-label="Close system tray" onClick={onClose}><X size={17} /></button>
    </header>
    <div className="tray-scroll">
      {icons?.length ? <ul className="tray-entries" aria-label="Running tray apps">
        {icons.map((item) => {
          const name = item.label.trim().split(/\r?\n/, 1)[0];
          const target = { app: item, name };
          return <li key={item.id} className="tray-entry"
            onContextMenu={(event) => contextMenu(event, target)}
            onKeyDown={(event) => menuShortcut(event, target)}>
            <button type="button" className="tray-entry-app" title={item.label}
              onClick={() => void invoke('activate_tray_icon', { id: item.id }).catch(onError)}>
              <span className="tray-entry-icon">
                {item.icon ? <img src={item.icon} alt="" draggable={false} /> : <AppWindow size={18} aria-hidden="true" />}
              </span>
              <span className="tray-entry-name">{name}</span>
            </button>
            <button type="button" className="tray-entry-menu" aria-haspopup="menu" aria-expanded={menu?.app?.id === item.id}
              aria-label={`Open ${name} menu`} title={`Open ${name} menu`}
              onClick={(event) => openMenu(target, event.currentTarget)}><Ellipsis size={19} aria-hidden="true" /></button>
          </li>;
        })}
      </ul> : <p className="tray-status" role="status">
        {failure || (icons ? 'No tray apps are active.' : 'Loading tray apps…')}
      </p>}
    </div>
    <footer className="tray-footer">
      <span>Right-click an item for options</span>
      <button type="button" aria-haspopup="menu" aria-expanded={!!menu && !menu.app}
        onClick={(event) => openMenu({ name: 'USB drives' }, event.currentTarget)}
        onContextMenu={(event) => contextMenu(event, { name: 'USB drives' })}
        onKeyDown={(event) => menuShortcut(event, { name: 'USB drives' })}><Usb size={15} aria-hidden="true" />USB drives</button>
    </footer>
    {menu && <div className="dock-context tray-context" role="menu" aria-label={menu.name} tabIndex={-1} ref={menuElement}
      onContextMenu={(event) => event.preventDefault()} onKeyDown={moveFocus}>
      <h2 className="tray-context-title" aria-hidden="true">{menu.name}</h2>
      {menu.app ? <>
        <button type="button" role="menuitem" disabled={busy} onClick={() => void run('activate_tray_icon', menu.app.id)}>
          {menu.app.isSpacebar ? <Settings2 size={15} /> : <AppWindow size={15} />}{menu.app.isSpacebar ? 'Open Settings' : 'Open app'}
        </button>
        {menu.app.canReveal && <button type="button" role="menuitem" disabled={busy} onClick={() => void run('reveal_tray_app', menu.app.id)}>
          <FolderOpen size={15} />Show in File Explorer
        </button>}
        {menu.app.isSpacebar && <button type="button" role="menuitem" className="context-quit" disabled={busy}
          onClick={() => void run('quit_spacebar')}><LogOut size={15} />Quit Spacebar</button>}
      </> : <>
        {drives?.map((drive) => <button type="button" key={drive.id} role="menuitem" disabled={busy}
          onClick={() => void run('eject_tray_usb_drive', drive.id, drive)}>
          <Usb size={16} /><span className="tray-drive-label"><strong>Eject {drive.label}</strong>
            {drive.letters.length > 0 && <small>{drive.letters.join(', ')}</small>}</span>
        </button>)}
        {!drives?.length && !menuError && !message && <p className="tray-context-status" role="status">
          {drives ? 'No ejectable USB drives connected.' : 'Looking for USB drives…'}
        </p>}
      </>}
      {busy && <p className="tray-context-status" role="status">{menu.app ? 'Opening…' : 'Ejecting drive…'}</p>}
      {message && <p className="tray-context-status" role="status"><Check size={15} />{message}</p>}
      {menuError && <p className="context-error" role="alert">{menuError}</p>}
    </div>}
  </>;
}
