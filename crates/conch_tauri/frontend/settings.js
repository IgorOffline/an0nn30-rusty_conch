// Settings Dialog — sidebar navigation, content area switching, Apply/Cancel.

(function (exports) {
  'use strict';

  let invoke = null;
  let listenFn = null;
  let escapeHandler = null;
  let currentSection = 'appearance';
  let pendingSettings = null;
  let originalSettings = null;
  let cachedThemes = [];
  let cachedPlugins = [];

  const SECTIONS = [
    { group: 'General', items: [
      { id: 'appearance', label: 'Appearance' },
      { id: 'keyboard', label: 'Keyboard Shortcuts' },
    ]},
    { group: 'Editor', items: [
      { id: 'terminal', label: 'Terminal' },
      { id: 'shell', label: 'Shell' },
      { id: 'cursor', label: 'Cursor' },
    ]},
    { group: 'Extensions', items: [
      { id: 'plugins', label: 'Plugins' },
    ]},
    { group: 'Advanced', items: [
      { id: 'advanced', label: 'Advanced' },
    ]},
  ];

  function init(opts) {
    invoke = opts.invoke;
    listenFn = opts.listen;
  }

  async function open() {
    if (document.getElementById('settings-overlay')) { close(); return; }

    try {
      const [settings, themes, plugins] = await Promise.all([
        invoke('get_all_settings'),
        invoke('list_themes'),
        invoke('scan_plugins'),
      ]);
      originalSettings = JSON.parse(JSON.stringify(settings));
      pendingSettings = JSON.parse(JSON.stringify(settings));
      cachedThemes = themes;
      cachedPlugins = plugins;
      currentSection = 'appearance';
      renderDialog();
    } catch (e) {
      if (window.toast) window.toast.error('Settings', 'Failed to load settings: ' + e);
    }
  }

  function close() {
    const el = document.getElementById('settings-overlay');
    if (el) el.remove();
    if (escapeHandler) {
      document.removeEventListener('keydown', escapeHandler, true);
      escapeHandler = null;
    }
    pendingSettings = null;
    originalSettings = null;
  }

  function renderDialog() {
    const overlay = document.createElement('div');
    overlay.className = 'ssh-overlay';
    overlay.id = 'settings-overlay';

    const dialog = document.createElement('div');
    dialog.className = 'ssh-form settings-dialog';

    // Title
    const title = document.createElement('div');
    title.className = 'ssh-form-title';
    title.textContent = 'Settings';
    dialog.appendChild(title);

    // Body = sidebar + content
    const body = document.createElement('div');
    body.className = 'settings-body';

    // Sidebar
    const sidebar = document.createElement('div');
    sidebar.className = 'settings-sidebar';
    sidebar.id = 'settings-sidebar';
    for (const group of SECTIONS) {
      const groupEl = document.createElement('div');
      groupEl.className = 'settings-sidebar-group';
      groupEl.textContent = group.group;
      sidebar.appendChild(groupEl);
      for (const item of group.items) {
        const itemEl = document.createElement('div');
        itemEl.className = 'settings-sidebar-item' + (item.id === currentSection ? ' active' : '');
        itemEl.textContent = item.label;
        itemEl.dataset.section = item.id;
        itemEl.addEventListener('click', () => selectSection(item.id));
        sidebar.appendChild(itemEl);
      }
    }
    body.appendChild(sidebar);

    // Content area
    const content = document.createElement('div');
    content.className = 'settings-content';
    content.id = 'settings-content';
    body.appendChild(content);

    dialog.appendChild(body);

    // Footer
    const footer = document.createElement('div');
    footer.className = 'ssh-form-buttons settings-footer';
    const cancelBtn = document.createElement('button');
    cancelBtn.className = 'ssh-form-btn';
    cancelBtn.textContent = 'Cancel';
    cancelBtn.addEventListener('click', close);
    const applyBtn = document.createElement('button');
    applyBtn.className = 'ssh-form-btn primary';
    applyBtn.textContent = 'Apply';
    applyBtn.addEventListener('click', applySettings);
    footer.appendChild(cancelBtn);
    footer.appendChild(applyBtn);
    dialog.appendChild(footer);

    overlay.appendChild(dialog);

    // Click outside to close
    overlay.addEventListener('mousedown', (e) => { if (e.target === overlay) close(); });

    document.body.appendChild(overlay);

    // Escape handler (capture phase, before xterm.js)
    escapeHandler = function (e) {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        close();
      }
    };
    document.addEventListener('keydown', escapeHandler, true);

    // Render initial section
    renderCurrentSection();
  }

  function selectSection(id) {
    currentSection = id;
    // Update sidebar active state
    const sidebar = document.getElementById('settings-sidebar');
    if (sidebar) {
      for (const item of sidebar.querySelectorAll('.settings-sidebar-item')) {
        item.classList.toggle('active', item.dataset.section === id);
      }
    }
    renderCurrentSection();
  }

  function renderCurrentSection() {
    const content = document.getElementById('settings-content');
    if (!content) return;
    content.innerHTML = '';

    switch (currentSection) {
      case 'appearance': renderAppearance(content); break;
      case 'keyboard': renderKeyboard(content); break;
      case 'terminal': renderTerminal(content); break;
      case 'shell': renderShell(content); break;
      case 'cursor': renderCursor(content); break;
      case 'plugins': renderPlugins(content); break;
      case 'advanced': renderAdvanced(content); break;
    }
  }

  // Placeholder section renderers — implemented in Tasks 6-8
  function renderAppearance(c) {
    c.innerHTML = '<h3>Appearance</h3><p style="color:var(--dim-fg)">Coming soon...</p>';
  }
  function renderKeyboard(c) {
    c.innerHTML = '<h3>Keyboard Shortcuts</h3><p style="color:var(--dim-fg)">Coming soon...</p>';
  }
  function renderTerminal(c) {
    c.innerHTML = '<h3>Terminal</h3><p style="color:var(--dim-fg)">Coming soon...</p>';
  }
  function renderShell(c) {
    c.innerHTML = '<h3>Shell</h3><p style="color:var(--dim-fg)">Coming soon...</p>';
  }
  function renderCursor(c) {
    c.innerHTML = '<h3>Cursor</h3><p style="color:var(--dim-fg)">Coming soon...</p>';
  }
  function renderPlugins(c) {
    c.innerHTML = '<h3>Plugins</h3><p style="color:var(--dim-fg)">Coming soon...</p>';
  }
  function renderAdvanced(c) {
    c.innerHTML = '<h3>Advanced</h3><p style="color:var(--dim-fg)">Coming soon...</p>';
  }

  async function applySettings() {
    try {
      const result = await invoke('save_settings', { settings: pendingSettings });
      close();
      if (result && result.restart_required) {
        window.toast && window.toast.show('Some changes require a restart to take effect', 'info', 5000);
      }
    } catch (e) {
      window.toast && window.toast.show('Failed to save settings: ' + e, 'error');
    }
  }

  exports.settings = { init, open, close };
})(window);
