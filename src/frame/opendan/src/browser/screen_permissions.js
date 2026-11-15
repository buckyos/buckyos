// Runs in the page's own world: the calls that ask the user for a permission
// also raise an event the screen's script hears. The browser refuses the
// permission either way; this only lets the user know it was asked for.
(() => {
  const mark = Symbol.for('opendan.permissions');
  if (window[mark]) return;
  Object.defineProperty(window, mark, { value: true });
  const tell = (name) => {
    try { document.dispatchEvent(new Event('opendan-permission-' + name)); } catch (_) { /* no document yet */ }
  };
  const wrap = (owner, method, names) => {
    const original = owner && owner[method];
    if (typeof original !== 'function') return;
    const wrapped = {
      [method](...args) {
        for (const name of names(args)) tell(name);
        return original.apply(this, args);
      },
    }[method];
    try {
      Object.defineProperty(owner, method, { ...Object.getOwnPropertyDescriptor(owner, method), value: wrapped });
    } catch (_) { /* not configurable here */ }
  };
  if (window.Geolocation) {
    wrap(Geolocation.prototype, 'getCurrentPosition', () => ['geolocation']);
    wrap(Geolocation.prototype, 'watchPosition', () => ['geolocation']);
  }
  if (window.MediaDevices) {
    wrap(MediaDevices.prototype, 'getUserMedia', (args) => {
      const wanted = args[0] || {};
      return [wanted.video ? 'camera' : null, wanted.audio ? 'microphone' : null].filter(Boolean);
    });
  }
  if (window.Notification) wrap(Notification, 'requestPermission', () => ['notifications']);
  if (window.Clipboard) {
    wrap(Clipboard.prototype, 'readText', () => ['clipboard-read']);
    wrap(Clipboard.prototype, 'read', () => ['clipboard-read']);
  }
  if (window.Navigator && Navigator.prototype.requestMIDIAccess) wrap(Navigator.prototype, 'requestMIDIAccess', () => ['midi']);
})();
