// Runs in every document of the agent's browser, in a world of its own
// (page scripts see neither it nor `__opendanScreen`): what the screen cannot
// show as part of a frame is told to the Agent Service. In a headed browser
// (`__opendanHeaded`) the display shows the browser's own lists and prompts:
// the script only listens, telling where the focused field is on the display.
(() => {
  if (globalThis.__opendanScreenInstalled) return;
  globalThis.__opendanScreenInstalled = true;
  const headed = Boolean(globalThis.__opendanHeaded);
  const say = (message) => {
    try { globalThis.__opendanScreen(JSON.stringify(message)); } catch (_) { /* the service is not listening */ }
  };

  // A client rect of this document, in the coordinates of the frames and
  // of the input: the top page's visual viewport.
  const place = (rect) => {
    let x = rect.left;
    let y = rect.top;
    let win = window;
    try {
      while (win !== win.parent && win.frameElement) {
        const outer = win.frameElement.getBoundingClientRect();
        x += outer.left + win.frameElement.clientLeft;
        y += outer.top + win.frameElement.clientTop;
        win = win.parent;
      }
    } catch (_) { /* an ancestor of another origin: its offset is unknown */ }
    const view = win.visualViewport;
    const scale = view ? view.scale : 1;
    // On the display: the page's place in its window, the window's on the display.
    const left = headed ? win.screenX + Math.max(0, (win.outerWidth - win.innerWidth) / 2) : 0;
    const top = headed ? win.screenY + Math.max(0, win.outerHeight - win.innerHeight) : 0;
    return {
      x: Math.round(left + (x - (view ? view.offsetLeft : 0)) * scale),
      y: Math.round(top + (y - (view ? view.offsetTop : 0)) * scale),
      width: Math.round(rect.width * scale),
      height: Math.round(rect.height * scale),
    };
  };

  // ---- <select>: its list is drawn outside the page, so the screen shows it. Headless only.
  const isMenu = (el) => el instanceof HTMLSelectElement && !el.multiple && el.size <= 1 && !el.disabled;
  const opened = new Map();
  let tokens = 0;
  const open = (el) => {
    const token = ++tokens;
    opened.clear();
    opened.set(token, el);
    const options = [];
    for (let i = 0; i < el.options.length && i < 2000; i++) {
      const option = el.options[i];
      const group = option.parentElement instanceof HTMLOptGroupElement ? option.parentElement : null;
      const item = {
        index: i,
        label: String(option.label || option.text || '').slice(0, 500),
        selected: option.selected,
        disabled: option.disabled || Boolean(group && group.disabled),
      };
      if (group) item.group = String(group.label || '').slice(0, 200);
      options.push(item);
    }
    say({ kind: 'select', token, multiple: el.multiple, options, rect: place(el.getBoundingClientRect()) });
  };
  if (!headed) document.addEventListener('mousedown', (event) => {
    if (event.button !== 0) return;
    const path = event.composedPath ? event.composedPath() : [event.target];
    const el = path.find((node) => node instanceof HTMLSelectElement);
    if (!el || !isMenu(el)) return;
    event.preventDefault();
    el.focus();
    open(el);
  }, true);
  if (!headed) document.addEventListener('keydown', (event) => {
    const el = event.target;
    if (!isMenu(el)) return;
    const opens = event.key === ' ' || event.key === 'Enter' || event.key === 'F4'
      || ((event.key === 'ArrowDown' || event.key === 'ArrowUp') && event.altKey);
    if (!opens) return;
    event.preventDefault();
    open(el);
  }, true);
  globalThis.__opendanSelect = (token, indexes) => {
    const el = opened.get(token);
    opened.delete(token);
    if (!el || !el.isConnected) return false;
    let changed = false;
    if (el.multiple) {
      for (let i = 0; i < el.options.length; i++) {
        const on = indexes.includes(i);
        if (el.options[i].selected !== on) { el.options[i].selected = on; changed = true; }
      }
    } else if (indexes.length && indexes[0] >= 0 && indexes[0] < el.options.length && el.selectedIndex !== indexes[0]) {
      el.selectedIndex = indexes[0];
      changed = true;
    }
    if (changed) {
      el.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
    }
    return changed;
  };

  // ---- focus: whether the focused element takes text (the App shows its keyboard).
  const TEXT_INPUTS = new Set(['text', 'search', 'url', 'tel', 'email', 'password', 'number', 'date', 'time', 'datetime-local', 'month', 'week']);
  const blurred = JSON.stringify({ kind: 'focus', editable: false });
  let told = blurred;
  const report = () => {
    let el = document.activeElement;
    while (el && el.shadowRoot && el.shadowRoot.activeElement) el = el.shadowRoot.activeElement;
    // A frame tells about its own document.
    if (el instanceof HTMLIFrameElement || el instanceof HTMLFrameElement) return;
    let message = { kind: 'focus', editable: false };
    if (el && document.hasFocus() && !el.disabled && !el.readOnly) {
      let type = null;
      if (el instanceof HTMLTextAreaElement) type = 'textarea';
      else if (el instanceof HTMLInputElement) type = TEXT_INPUTS.has(el.type) ? el.type : null;
      else if (el.isContentEditable) type = 'contenteditable';
      if (type) message = { kind: 'focus', editable: true, input_type: type, rect: place(el.getBoundingClientRect()) };
    }
    const text = JSON.stringify(message);
    if (text === told) return;
    told = text;
    say(message);
  };
  for (const type of ['focusin', 'focusout']) document.addEventListener(type, () => setTimeout(report, 0), true);
  report();
})();
