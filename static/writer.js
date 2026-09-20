const csrf = document.querySelector('meta[name="csrf-token"]').content;
const editor = document.querySelector('#editor');
const emptyState = document.querySelector('#empty-state');
const list = document.querySelector('#article-list');
const preview = document.querySelector('#preview');
const assetList = document.querySelector('#asset-list');
const toastNode = document.querySelector('#toast');
const splitEditor = document.querySelector('.split-editor');
const writerShell = document.querySelector('.writer-shell');
const sidebarToggle = document.querySelector('#sidebar-toggle');
const metadataPanel = document.querySelector('#metadata-panel');
const metadataToggle = document.querySelector('#metadata-toggle');
const publishAction = document.querySelector('#publish-action');
const bodyInput = editor.elements.body;
let articles = [];
let currentSlug = null;
let previewTimer = null;
let previewGeneration = 0;
let dirty = false;
let bodyHistory = [];
let bodyHistoryIndex = -1;
let historyTimer = null;
let applyingHistory = false;

function toast(message, error = false) {
  toastNode.textContent = message;
  toastNode.className = `show${error ? ' error' : ''}`;
  clearTimeout(toastNode.timer);
  toastNode.timer = setTimeout(() => toastNode.className = '', 3000);
}

async function api(url, options = {}) {
  const headers = new Headers(options.headers || {});
  headers.set('X-CSRF-Token', csrf);
  if (options.body && !(options.body instanceof FormData)) headers.set('Content-Type', 'application/json');
  const response = await fetch(url, {...options, headers});
  const data = await response.json().catch(() => ({}));
  if (response.status === 401) {
    window.location.assign('/writer/login');
    throw new Error('Session expired');
  }
  if (!response.ok) throw new Error(data.error || `Request failed (${response.status})`);
  return data;
}

function localDateTime(iso) {
  const date = new Date(iso);
  const offset = date.getTimezoneOffset() * 60000;
  return new Date(date - offset).toISOString().slice(0, 16);
}

function formData() {
  const data = Object.fromEntries(new FormData(editor));
  data.published = editor.elements.published.checked;
  data.draft = !data.published;
  return data;
}

function markDirty() {
  dirty = true;
  document.querySelector('#dirty-dot').classList.add('dirty');
  document.querySelector('#save-message').textContent = 'Unsaved';
  clearTimeout(previewTimer);
  previewTimer = setTimeout(refreshPreview, 250);
  updateEditorStatus();
}

function markSaved() {
  dirty = false;
  document.querySelector('#dirty-dot').classList.remove('dirty');
  document.querySelector('#save-message').textContent = 'Saved';
}

function editorialStatus(article) {
  if (!article.published) return 'Draft';
  return new Date(article.release_date) > new Date() ? 'Scheduled' : 'Published';
}

function updateEditorStatus() {
  if (!currentSlug) return;
  const data = formData();
  document.querySelector('#editor-status').textContent = editorialStatus(data);
  updatePublishAction(data);
}

function updatePublishAction(article) {
  const releasesLater = new Date(article.release_date) > new Date();
  const action = article.published ? 'unpublish' : releasesLater ? 'schedule' : 'publish';
  const label = action === 'unpublish' ? 'Unpublish' : action === 'schedule' ? 'Schedule' : 'Publish';
  publishAction.dataset.action = action;
  publishAction.textContent = label;
  publishAction.title = action === 'unpublish'
    ? 'Save this article as a draft'
    : `${label} this article and save all changes`;
}

function updatePublicLink(article) {
  const publicLink = document.querySelector('#public-link');
  const publiclyVisible = editorialStatus(article) === 'Published';
  if (publiclyVisible) publicLink.href = article.url;
  else publicLink.removeAttribute('href');
  publicLink.classList.toggle('disabled', !publiclyVisible);
  publicLink.setAttribute('aria-disabled', String(!publiclyVisible));
  publicLink.title = publiclyVisible ? 'Open the public article' : 'This article is not public yet';
}

function mayDiscardChanges() {
  return !dirty || window.confirm('Discard the unsaved changes to this article?');
}

function setSidebarOpen(open) {
  writerShell.classList.toggle('sidebar-open', open);
  sidebarToggle.setAttribute('aria-expanded', String(open));
}

function storedMetadataState() {
  try {
    const current = window.localStorage.getItem('mf-blog.metadataCollapsed');
    return current === 'true';
  }
  catch (_error) { return false; }
}

function setMetadataCollapsed(collapsed, persist = true) {
  metadataPanel.hidden = collapsed;
  editor.classList.toggle('metadata-collapsed', collapsed);
  metadataToggle.setAttribute('aria-expanded', String(!collapsed));
  metadataToggle.textContent = collapsed ? 'Metadata ↓' : 'Metadata ↑';
  metadataToggle.title = `${collapsed ? 'Show' : 'Hide'} metadata (Ctrl+Shift+M)`;
  if (persist) {
    try { window.localStorage.setItem('mf-blog.metadataCollapsed', String(collapsed)); }
    catch (_error) { /* The preference is optional when storage is unavailable. */ }
  }
}

function bodySnapshot() {
  return {value: bodyInput.value, start: bodyInput.selectionStart, end: bodyInput.selectionEnd};
}

function updateHistoryButtons() {
  const undo = document.querySelector('[data-command="undo"]');
  const redo = document.querySelector('[data-command="redo"]');
  undo.disabled = bodyHistoryIndex <= 0;
  redo.disabled = bodyHistoryIndex >= bodyHistory.length - 1;
}

function resetBodyHistory() {
  clearTimeout(historyTimer);
  bodyHistory = [bodySnapshot()];
  bodyHistoryIndex = 0;
  updateHistoryButtons();
}

function commitBodyHistory() {
  clearTimeout(historyTimer);
  const snapshot = bodySnapshot();
  const current = bodyHistory[bodyHistoryIndex];
  if (current?.value === snapshot.value) {
    current.start = snapshot.start;
    current.end = snapshot.end;
    updateHistoryButtons();
    return;
  }
  bodyHistory = bodyHistory.slice(0, bodyHistoryIndex + 1);
  bodyHistory.push(snapshot);
  if (bodyHistory.length > 100) bodyHistory.shift();
  bodyHistoryIndex = bodyHistory.length - 1;
  updateHistoryButtons();
}

function queueBodyHistory() {
  if (applyingHistory) return;
  clearTimeout(historyTimer);
  historyTimer = setTimeout(commitBodyHistory, 350);
}

function applyHistory(index) {
  if (index < 0 || index >= bodyHistory.length) return;
  clearTimeout(historyTimer);
  bodyHistoryIndex = index;
  const snapshot = bodyHistory[index];
  applyingHistory = true;
  bodyInput.value = snapshot.value;
  bodyInput.setSelectionRange(snapshot.start, snapshot.end);
  applyingHistory = false;
  bodyInput.focus();
  markDirty();
  updateHistoryButtons();
}

function undoBody() {
  commitBodyHistory();
  if (bodyHistoryIndex > 0) applyHistory(bodyHistoryIndex - 1);
}

function redoBody() {
  if (bodyHistoryIndex < bodyHistory.length - 1) applyHistory(bodyHistoryIndex + 1);
}

function editBody(change) {
  commitBodyHistory();
  change();
  bodyInput.dispatchEvent(new Event('input', {bubbles: true}));
  commitBodyHistory();
  bodyInput.focus();
}

function wrapSelection(before, after = before, placeholder = 'text') {
  editBody(() => {
    const start = bodyInput.selectionStart;
    const end = bodyInput.selectionEnd;
    const selected = bodyInput.value.slice(start, end) || placeholder;
    bodyInput.setRangeText(`${before}${selected}${after}`, start, end, 'end');
    bodyInput.setSelectionRange(start + before.length, start + before.length + selected.length);
  });
}

function prefixSelectedLines(prefix) {
  editBody(() => {
    const start = bodyInput.value.lastIndexOf('\n', bodyInput.selectionStart - 1) + 1;
    const nextBreak = bodyInput.value.indexOf('\n', bodyInput.selectionEnd);
    const end = nextBreak === -1 ? bodyInput.value.length : nextBreak;
    const replacement = bodyInput.value.slice(start, end).split('\n').map(line => `${prefix}${line}`).join('\n');
    bodyInput.setRangeText(replacement, start, end, 'select');
  });
}

const markdownCommands = {
  undo: undoBody,
  redo: redoBody,
  bold: () => wrapSelection('**'),
  italic: () => wrapSelection('_'),
  link: () => wrapSelection('[', '](url)', 'link text'),
  code: () => wrapSelection('`'),
  heading: () => prefixSelectedLines('## '),
  quote: () => prefixSelectedLines('> '),
  list: () => prefixSelectedLines('- ')
};

function renderList() {
  list.innerHTML = '';
  for (const article of articles) {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = article.slug === currentSlug ? 'active' : '';
    const status = editorialStatus(article);
    button.innerHTML = `<strong></strong><span></span>`;
    button.querySelector('strong').textContent = article.title;
    button.querySelector('span').textContent = `${status} · ${article.slug}`;
    button.addEventListener('click', async () => {
      await openArticle(article.slug);
      if (currentSlug === article.slug) setSidebarOpen(false);
    });
    list.append(button);
  }
}

function renderAssets(assets) {
  assetList.innerHTML = '';
  for (const asset of assets) {
    const button = document.createElement('button');
    button.type = 'button';
    button.textContent = asset;
    button.title = 'Insert Markdown image';
    button.addEventListener('click', () => insertAtCursor(`![${asset}](assets/${asset})`));
    assetList.append(button);
  }
}

function insertAtCursor(text) {
  editBody(() => bodyInput.setRangeText(text, bodyInput.selectionStart, bodyInput.selectionEnd, 'end'));
}

async function loadArticles(selectSlug = null) {
  const data = await api('/writer/api/articles');
  articles = data.articles;
  renderList();
  if (selectSlug) await openArticle(selectSlug);
  if (data.errors.length) toast(data.errors[0], true);
}

async function openArticle(slug) {
  if (slug !== currentSlug && !mayDiscardChanges()) return;
  const article = await api(`/writer/api/articles/${encodeURIComponent(slug)}`);
  currentSlug = article.slug;
  editor.hidden = false;
  emptyState.hidden = true;
  for (const name of ['slug', 'title', 'subtitle', 'hero', 'body']) editor.elements[name].value = article[name] || '';
  editor.elements.publication_date.value = localDateTime(article.publication_date);
  editor.elements.release_date.value = localDateTime(article.release_date);
  editor.elements.published.checked = article.published;
  editor.elements.slug.readOnly = true;
  updatePublicLink(article);
  renderAssets(article.assets);
  renderList();
  resetBodyHistory();
  markSaved();
  updateEditorStatus();
  await refreshPreview();
}

async function refreshPreview() {
  if (!currentSlug) return;
  const generation = ++previewGeneration;
  document.querySelector('#preview-state').textContent = 'Rendering…';
  try {
    const data = await api('/writer/api/preview', {method: 'POST', body: JSON.stringify(formData())});
    if (generation === previewGeneration) {
      preview.innerHTML = data.html;
      document.querySelector('#preview-state').textContent = 'Live';
    }
  } catch (error) {
    if (generation === previewGeneration) document.querySelector('#preview-state').textContent = error.message;
  }
}

editor.addEventListener('input', markDirty);
editor.addEventListener('change', markDirty);
bodyInput.addEventListener('input', queueBodyHistory);
editor.addEventListener('submit', async event => {
  event.preventDefault();
  try {
    const saved = await api(`/writer/api/articles/${encodeURIComponent(currentSlug)}`, {method: 'PUT', body: JSON.stringify(formData())});
    const index = articles.findIndex(item => item.slug === currentSlug);
    articles[index] = saved;
    renderList();
    renderAssets(saved.assets);
    updatePublicLink(saved);
    markSaved();
    updateEditorStatus();
    toast('Article saved');
  } catch (error) { toast(error.message, true); }
});

publishAction.addEventListener('click', () => {
  editor.elements.published.checked = !editor.elements.published.checked;
  markDirty();
  editor.requestSubmit();
});

const newDialog = document.querySelector('#new-dialog');
document.querySelector('#new-article').addEventListener('click', () => {
  if (mayDiscardChanges()) newDialog.showModal();
});
document.querySelector('#new-form').addEventListener('submit', async event => {
  event.preventDefault();
  const submitter = event.submitter;
  if (submitter?.value === 'cancel') { newDialog.close(); return; }
  const input = Object.fromEntries(new FormData(event.currentTarget));
  const now = new Date();
  try {
    const article = await api('/writer/api/articles', {method: 'POST', body: JSON.stringify({
      ...input, subtitle: '', hero: '', body: '# Start writing\n', draft: true,
      publication_date: now.toISOString(), release_date: now.toISOString()
    })});
    newDialog.close();
    event.currentTarget.reset();
    await loadArticles(article.slug);
    toast('Article created');
  } catch (error) { toast(error.message, true); }
});

const deleteDialog = document.querySelector('#delete-dialog');
document.querySelector('#delete-article').addEventListener('click', () => {
  document.querySelector('#delete-title').textContent = editor.elements.title.value || currentSlug;
  deleteDialog.showModal();
});
document.querySelector('#delete-form').addEventListener('submit', async event => {
  event.preventDefault();
  if (event.submitter?.value !== 'delete') { deleteDialog.close(); return; }
  try {
    const result = await api(`/writer/api/articles/${encodeURIComponent(currentSlug)}`, {method: 'DELETE'});
    deleteDialog.close();
    currentSlug = null;
    editor.hidden = true;
    emptyState.hidden = false;
    await loadArticles();
    toast(`Moved to trash: ${result.trash_name}`);
  } catch (error) { toast(error.message, true); }
});

document.querySelector('#asset-input').addEventListener('change', async event => {
  const form = new FormData();
  for (const file of event.target.files) form.append('files', file, file.name);
  try {
    await api(`/writer/api/articles/${encodeURIComponent(currentSlug)}/assets`, {method: 'POST', body: form});
    await openArticle(currentSlug);
    toast('Assets uploaded');
  } catch (error) { toast(error.message, true); }
  event.target.value = '';
});

async function importFiles(files) {
  const form = new FormData();
  for (const file of files) form.append('files', file, file.relativeUploadPath || file.webkitRelativePath || file.name);
  try {
    const article = await api('/writer/api/import', {method: 'POST', body: form});
    await loadArticles(article.slug);
    toast('Article imported');
  } catch (error) { toast(error.message, true); }
}

async function filesFromDrop(items) {
  const files = [];
  async function walk(entry, prefix = '') {
    if (entry.isFile) {
      const file = await new Promise((resolve, reject) => entry.file(resolve, reject));
      Object.defineProperty(file, 'relativeUploadPath', {value: `${prefix}${file.name}`});
      files.push(file);
      return;
    }
    if (entry.isDirectory) {
      const reader = entry.createReader();
      let entries;
      do {
        entries = await new Promise((resolve, reject) => reader.readEntries(resolve, reject));
        for (const child of entries) await walk(child, `${prefix}${entry.name}/`);
      } while (entries.length);
    }
  }
  for (const item of items) {
    const entry = item.webkitGetAsEntry?.();
    if (entry) await walk(entry);
    else {
      const file = item.getAsFile?.();
      if (file) files.push(file);
    }
  }
  return files;
}

document.querySelector('#zip-input').addEventListener('change', event => importFiles(event.target.files));
document.querySelector('#folder-input').addEventListener('change', event => importFiles(event.target.files));
sidebarToggle.addEventListener('click', () => setSidebarOpen(!writerShell.classList.contains('sidebar-open')));
document.querySelector('#sidebar-backdrop').addEventListener('click', () => setSidebarOpen(false));
document.querySelector('#logout-button').addEventListener('click', async () => {
  if (!mayDiscardChanges()) return;
  try {
    await api('/writer/logout', {method: 'POST'});
    window.location.assign('/writer/login');
  } catch (error) { toast(error.message, true); }
});
const drop = document.querySelector('#import-drop');
for (const name of ['dragenter', 'dragover']) drop.addEventListener(name, event => { event.preventDefault(); drop.classList.add('dragging'); });
for (const name of ['dragleave', 'drop']) drop.addEventListener(name, event => { event.preventDefault(); drop.classList.remove('dragging'); });
drop.addEventListener('drop', async event => {
  try { await importFiles(await filesFromDrop(event.dataTransfer.items)); }
  catch (error) { toast(error.message, true); }
});

for (const button of document.querySelectorAll('.mobile-pane-switch button')) {
  button.addEventListener('click', () => {
    const showPreview = button.dataset.pane === 'preview';
    splitEditor.classList.toggle('show-preview', showPreview);
    for (const candidate of document.querySelectorAll('.mobile-pane-switch button')) {
      candidate.classList.toggle('active', candidate === button);
    }
    if (showPreview) refreshPreview();
  });
}

metadataToggle.addEventListener('click', () => setMetadataCollapsed(!metadataPanel.hidden));
for (const button of document.querySelectorAll('.markdown-tools button')) {
  button.addEventListener('mousedown', event => event.preventDefault());
  button.addEventListener('click', () => markdownCommands[button.dataset.command]());
}
setMetadataCollapsed(storedMetadataState(), false);

loadArticles().catch(error => toast(error.message, true));

document.addEventListener('keydown', event => {
  if (event.key === 'Escape' && writerShell.classList.contains('sidebar-open')) {
    setSidebarOpen(false);
    sidebarToggle.focus();
    return;
  }
  const modifier = event.ctrlKey || event.metaKey;
  const key = event.key.toLowerCase();
  if (modifier && event.shiftKey && key === 'm' && !editor.hidden) {
    event.preventDefault();
    setMetadataCollapsed(!metadataPanel.hidden);
    return;
  }
  if (modifier && key === 's' && !editor.hidden) {
    event.preventDefault();
    editor.requestSubmit();
    return;
  }
  if (event.target !== bodyInput || !modifier) return;
  if (key === 'z') {
    event.preventDefault();
    if (event.shiftKey) redoBody();
    else undoBody();
  } else if (key === 'y') {
    event.preventDefault();
    redoBody();
  } else if (key === 'b' || key === 'i' || key === 'k') {
    event.preventDefault();
    markdownCommands[{b: 'bold', i: 'italic', k: 'link'}[key]]();
  }
});
window.addEventListener('beforeunload', event => {
  if (!dirty) return;
  event.preventDefault();
  event.returnValue = '';
});
