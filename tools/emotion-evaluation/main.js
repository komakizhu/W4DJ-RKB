import {
  HUMAN_LABELS,
  MODEL_IDS,
  createEvaluationSession,
  exportEvaluationCsv,
  exportEvaluationJson,
  matchRelativeAudioFiles,
  modelStatus,
  selectableModelIds,
  scoreSelection,
  summarizeSession,
} from './evaluator.js';

const state = {
  manifest: null,
  session: null,
  audioFiles: [],
  audioMap: new Map(),
  selectedHumanLabel: null,
  selectedWinners: new Set(),
  audioPlayable: false,
  currentObjectUrl: null,
};

const elements = Object.fromEntries([
  'manifest-input', 'audio-input', 'setup-status', 'start-button', 'resume-button',
  'setup', 'evaluation', 'summary', 'progress-text', 'pause-button', 'track-title',
  'previous-button',
  'track-artist', 'track-path', 'audio-player', 'audio-status', 'human-stage',
  'human-options', 'human-submit', 'model-stage', 'model-cards', 'tie-button',
  'none-button', 'model-submit', 'summary-output', 'edit-last-button', 'export-json', 'export-csv',
].map((id) => [id, document.getElementById(id)]));

const DB_NAME = 'w4dj-emotion-evaluation';
const DB_STORE = 'sessions';

function openDatabase() {
  return new Promise((resolve, reject) => {
    if (!('indexedDB' in window)) {
      resolve(null);
      return;
    }
    const request = indexedDB.open(DB_NAME, 1);
    request.onupgradeneeded = () => request.result.createObjectStore(DB_STORE, { keyPath: 'sessionId' });
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

async function saveSession() {
  if (!state.session) return;
  state.session.updatedAt = new Date().toISOString();
  const db = await openDatabase();
  if (db) {
    await new Promise((resolve, reject) => {
      const request = db.transaction(DB_STORE, 'readwrite').objectStore(DB_STORE).put(state.session);
      request.onsuccess = resolve;
      request.onerror = () => reject(request.error);
    });
  }
  localStorage.setItem(`w4dj-emotion-session:${state.session.sessionId}`, JSON.stringify(state.session));
}

async function readStoredSession(sessionId) {
  const db = await openDatabase();
  if (db) {
    const value = await new Promise((resolve, reject) => {
      const request = db.transaction(DB_STORE, 'readonly').objectStore(DB_STORE).get(sessionId);
      request.onsuccess = () => resolve(request.result ?? null);
      request.onerror = () => reject(request.error);
    });
    if (value) return value;
  }
  const raw = localStorage.getItem(`w4dj-emotion-session:${sessionId}`);
  return raw ? JSON.parse(raw) : null;
}

function downloadText(filename, content, type) {
  const blob = new Blob([content], { type });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = filename;
  anchor.click();
  setTimeout(() => URL.revokeObjectURL(url), 0);
}

function setSetupStatus(message, isError = false) {
  elements['setup-status'].textContent = message;
  elements['setup-status'].classList.toggle('error', isError);
}

function parseManifest(file) {
  return file.text().then((text) => {
    const manifest = JSON.parse(text);
    if (manifest.schemaVersion !== 1 || !Array.isArray(manifest.tracks)) {
      throw new Error('manifest schemaVersion 或 tracks 无效');
    }
    for (const track of manifest.tracks) {
      if (!track.trackId || !track.relativePath || !Number.isFinite(Number(track.clipStartSeconds))
        || !Number.isFinite(Number(track.clipDurationSeconds))) {
        throw new Error('manifest 中存在缺少路径或片段信息的歌曲');
      }
    }
    return manifest;
  });
}

function refreshSetupState() {
  const ready = state.manifest && state.audioFiles.length > 0;
  elements['start-button'].disabled = !ready;
  if (state.manifest) {
    setSetupStatus(`已加载 ${state.manifest.tracks.length} 首，已匹配 ${state.audioMap.size} 个音频文件。`);
  }
}

function currentTrack() {
  const trackId = state.session?.trackOrder[state.session.cursor];
  return state.manifest?.tracks.find((track) => track.trackId === trackId) ?? null;
}

function renderHumanOptions() {
  elements['human-options'].replaceChildren();
  for (const option of HUMAN_LABELS) {
    const button = document.createElement('button');
    button.className = 'choice-button';
    button.textContent = option.label;
    button.dataset.id = option.id;
    button.addEventListener('click', () => {
      state.selectedHumanLabel = option.id;
      elements['human-options'].querySelectorAll('button').forEach((item) => {
        item.classList.toggle('selected', item === button);
      });
      elements['human-submit'].disabled = false;
    });
    elements['human-options'].append(button);
  }
}

function outputText(modelId, value) {
  if (value?.status !== 'completed') return value?.status === 'failed' ? '模型分析失败' : '模型结果缺失';
  if (modelId === 'legacyMood' || modelId === 'mirex') {
    const labels = Array.isArray(value.labels) ? value.labels : [];
    return labels.length ? labels.map((label) => label.label ?? label).join(' · ') : '无高置信度标签';
  }
  return `愉悦度 ${Number(value.valence).toFixed(1)} · 激烈度 ${Number(value.arousal).toFixed(1)}`;
}

function renderTrack() {
  const track = currentTrack();
  if (!track) {
    finishSession();
    return;
  }
  const position = state.session.cursor + 1;
  elements['progress-text'].textContent = `${position} / ${state.session.trackOrder.length}`;
  elements['previous-button'].disabled = state.session.cursor === 0;
  elements['track-title'].textContent = track.title || track.relativePath;
  elements['track-artist'].textContent = track.artist || '未知艺术家';
  elements['track-path'].textContent = track.relativePath;
  const file = state.audioMap.get(track.relativePath.replaceAll('\\', '/'));
  state.audioPlayable = Boolean(file);
  if (state.currentObjectUrl) URL.revokeObjectURL(state.currentObjectUrl);
  state.currentObjectUrl = file ? URL.createObjectURL(file.file ?? file) : null;
  elements['audio-player'].src = state.currentObjectUrl ?? '';
  elements['audio-player'].dataset.start = String(track.clipStartSeconds ?? 0);
  elements['audio-player'].dataset.end = String((track.clipStartSeconds ?? 0) + (track.clipDurationSeconds ?? 10));
  elements['audio-status'].textContent = file ? '音频已匹配，播放后自动限制在 10 秒片段。' : '文件缺失：本首不会计入胜率。';
  state.selectedWinners = new Set();
  elements['model-submit'].disabled = true;
  if (state.session.phase === 'models' && state.session.pendingHumanLabel) {
    state.selectedHumanLabel = state.session.pendingHumanLabel;
    renderModelCards();
    elements['model-stage'].hidden = false;
    elements['human-stage'].hidden = true;
  } else {
    state.selectedHumanLabel = null;
    elements['human-options'].querySelectorAll('button').forEach((button) => button.classList.remove('selected'));
    elements['human-submit'].disabled = true;
    elements['model-stage'].hidden = true;
    elements['human-stage'].hidden = false;
  }
}

function renderModelCards() {
  const track = currentTrack();
  elements['model-cards'].replaceChildren();
  for (const modelId of state.session.cards[track.trackId]) {
    const card = document.createElement('button');
    card.className = 'model-card';
    card.dataset.modelId = modelId;
    card.innerHTML = `<span class="card-letter"></span><strong>模型结果</strong><span class="model-output"></span>`;
    card.querySelector('.model-output').textContent = outputText(modelId, track[modelId]);
    card.disabled = modelStatus(track, modelId) !== 'completed';
    card.addEventListener('click', () => {
      if (card.disabled) return;
      if (state.selectedWinners.has(modelId)) {
        state.selectedWinners.delete(modelId);
      } else {
        state.selectedWinners.add(modelId);
      }
      elements['model-cards'].querySelectorAll('button').forEach((item) => {
        state.selectedWinners.has(item.dataset.modelId)
          ? item.classList.add('selected')
          : item.classList.remove('selected');
      });
      elements['model-submit'].disabled = state.selectedWinners.size === 0;
    });
    elements['model-cards'].append(card);
  }
  elements['model-cards'].querySelectorAll('.card-letter').forEach((node, index) => {
    node.textContent = String.fromCharCode(65 + index);
  });
}

function submitHumanLabel() {
  if (!state.session || !state.selectedHumanLabel) return;
  state.session.phase = 'models';
  state.session.pendingHumanLabel = state.selectedHumanLabel;
  renderModelCards();
  elements['human-stage'].hidden = true;
  elements['model-stage'].hidden = false;
}

async function submitModelChoice(winnerIds) {
  const track = currentTrack();
  const availableModelIds = selectableModelIds(track);
  const answer = {
    humanLabel: state.session.pendingHumanLabel,
    availableModelIds,
    winnerIds,
    cardOrder: state.session.cards[track.trackId],
    submittedAt: new Date().toISOString(),
    audioMatched: state.audioPlayable,
  };
  state.session.answers[track.trackId] = answer;
  delete state.session.pendingHumanLabel;
  state.session.cursor += 1;
  state.session.phase = 'human';
  await saveSession();
  renderTrack();
}

function finishSession() {
  state.session.paused = false;
  elements['edit-last-button'].disabled = state.session.cursor === 0;
  elements['evaluation'].hidden = true;
  elements['summary'].hidden = false;
  elements['summary-output'].textContent = JSON.stringify(summarizeSession(state.session), null, 2);
}

elements['manifest-input'].addEventListener('change', async (event) => {
  try {
    state.manifest = await parseManifest(event.target.files[0]);
    refreshSetupState();
    const stored = await readStoredSession(state.manifest.sessionId);
    elements['resume-button'].hidden = !stored;
  } catch (error) {
    setSetupStatus(error instanceof Error ? error.message : String(error), true);
  }
});

elements['audio-input'].addEventListener('change', (event) => {
  state.audioFiles = [...event.target.files].map((file) => ({
    file,
    name: file.name,
    relativePath: file.webkitRelativePath || file.name,
  }));
  if (state.manifest) {
    state.audioMap = matchRelativeAudioFiles(
      state.audioFiles,
      state.manifest.tracks.map((track) => track.relativePath),
    );
  }
  refreshSetupState();
});

elements['start-button'].addEventListener('click', async () => {
  state.session = createEvaluationSession(state.manifest);
  await saveSession();
  elements['setup'].hidden = true;
  elements['evaluation'].hidden = false;
  renderHumanOptions();
  renderTrack();
});

elements['resume-button'].addEventListener('click', async () => {
  state.session = await readStoredSession(state.manifest.sessionId);
  if (!state.session) return;
  elements['setup'].hidden = true;
  elements['evaluation'].hidden = false;
  renderHumanOptions();
  renderTrack();
});

elements['human-submit'].addEventListener('click', submitHumanLabel);
elements['model-submit'].addEventListener('click', () => submitModelChoice([...state.selectedWinners]));
elements['tie-button'].addEventListener('click', () => {
  const available = selectableModelIds(currentTrack());
  if (available.length >= 2) {
    state.selectedWinners = new Set(available);
    submitModelChoice(available);
  }
});
elements['none-button'].addEventListener('click', () => submitModelChoice('none'));
elements['previous-button'].addEventListener('click', async () => {
  if (!state.session || state.session.cursor === 0) return;
  state.session.cursor -= 1;
  state.session.phase = 'human';
  delete state.session.pendingHumanLabel;
  await saveSession();
  renderTrack();
});
elements['pause-button'].addEventListener('click', async () => {
  state.session.paused = !state.session.paused;
  elements['pause-button'].textContent = state.session.paused ? '继续' : '暂停';
  await saveSession();
});
elements['edit-last-button'].addEventListener('click', async () => {
  if (!state.session || state.session.cursor === 0) return;
  state.session.cursor -= 1;
  state.session.phase = 'human';
  delete state.session.pendingHumanLabel;
  await saveSession();
  elements['summary'].hidden = true;
  elements['evaluation'].hidden = false;
  renderTrack();
});
elements['export-json'].addEventListener('click', () => {
  downloadText(`${state.session.sessionId}.json`, exportEvaluationJson(state.session), 'application/json');
});
elements['export-csv'].addEventListener('click', () => {
  downloadText(`${state.session.sessionId}.csv`, exportEvaluationCsv(state.session), 'text/csv;charset=utf-8');
});
elements['audio-player'].addEventListener('timeupdate', () => {
  const player = elements['audio-player'];
  const end = Number(player.dataset.end);
  if (Number.isFinite(end) && player.currentTime >= end) {
    player.pause();
    player.currentTime = Number(player.dataset.start) || 0;
  }
});
elements['audio-player'].addEventListener('loadedmetadata', () => {
  const player = elements['audio-player'];
  const start = Number(player.dataset.start);
  if (Number.isFinite(start) && start > 0) player.currentTime = start;
});
elements['audio-player'].addEventListener('error', () => {
  state.audioPlayable = false;
  elements['audio-status'].textContent = '音频无法播放：本首不会计入胜率。';
});
elements['audio-player'].addEventListener('play', () => {
  const player = elements['audio-player'];
  const start = Number(player.dataset.start);
  const end = Number(player.dataset.end);
  if (Number.isFinite(start) && player.currentTime < start) player.currentTime = start;
  if (Number.isFinite(end) && player.currentTime >= end) player.currentTime = start || 0;
});
