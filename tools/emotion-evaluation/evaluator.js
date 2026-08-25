export const MODEL_IDS = Object.freeze(['legacyMood', 'emomusic', 'muse', 'mirex']);

export const HUMAN_LABELS = Object.freeze([
  { id: 'bright', label: '明亮' },
  { id: 'sad', label: '悲伤' },
  { id: 'calm', label: '平静' },
  { id: 'intense', label: '激烈' },
  { id: 'excited', label: '兴奋' },
  { id: 'neutral', label: '中性 / 其他' },
  { id: 'uncertain', label: '无法判断' },
]);

const UINT32_MAX = 0x1_0000_0000;

export function normalizeRelativePath(value) {
  const source = String(value ?? '').replaceAll('\\', '/');
  const parts = [];
  for (const part of source.split('/')) {
    if (!part || part === '.') continue;
    if (part === '..') {
      parts.pop();
    } else {
      parts.push(part);
    }
  }
  return parts.join('/');
}

function nextRandom(state) {
  return (Math.imul(state, 1_664_525) + 1_013_904_223) >>> 0;
}

export function shuffleWithSeed(items, seed) {
  const result = [...items];
  let state = (Number(seed) >>> 0) || 0x9e37_79b9;
  for (let index = result.length - 1; index > 0; index -= 1) {
    state = nextRandom(state);
    const target = Math.floor((state / UINT32_MAX) * (index + 1));
    [result[index], result[target]] = [result[target], result[index]];
  }
  return result;
}

export function hashSeed(seed, value) {
  let hash = Number(seed) >>> 0;
  for (const character of String(value ?? '')) {
    hash = Math.imul(hash ^ character.codePointAt(0), 1_664_525) + 1_013_904_223;
    hash >>>= 0;
  }
  return hash >>> 0;
}

export function cardOrderForTrack(seed, trackId) {
  return shuffleWithSeed(MODEL_IDS, hashSeed(seed, trackId));
}

function relativeCandidates(path) {
  const normalized = normalizeRelativePath(path);
  const parts = normalized.split('/');
  return [normalized, parts.slice(1).join('/')].filter(Boolean);
}

export function matchRelativeAudioFiles(files, relativePaths) {
  const index = new Map();
  for (const file of files) {
    const candidates = relativeCandidates(file.relativePath ?? file.name);
    for (const candidate of candidates) {
      const existing = index.get(candidate);
      if (existing && existing !== file) {
        index.set(candidate, null);
      } else if (!index.has(candidate)) {
        index.set(candidate, file);
      }
    }
  }
  const matched = new Map();
  for (const relativePath of relativePaths) {
    const candidates = relativeCandidates(relativePath);
    const file = candidates.map((candidate) => index.get(candidate)).find(Boolean);
    if (file) matched.set(normalizeRelativePath(relativePath), file);
  }
  return matched;
}

export function modelStatus(track, modelId) {
  return track?.[modelId]?.status ?? 'missing';
}

export function selectableModelIds(track) {
  return MODEL_IDS.filter((modelId) => modelStatus(track, modelId) === 'completed');
}

export function scoreSelection(selection) {
  const available = MODEL_IDS.filter((modelId) => selection.availableModelIds?.includes(modelId));
  const winners = selection.winnerIds === 'none'
    ? []
    : [...new Set(selection.winnerIds ?? [])].filter((modelId) => available.includes(modelId));
  if (selection.audioMatched === false) {
    return {
      validSample: false,
      winnerIds: winners,
      points: Object.fromEntries(MODEL_IDS.map((modelId) => [modelId, 0])),
      denominator: Object.fromEntries(MODEL_IDS.map((modelId) => [modelId, 0])),
      reason: 'audio_unavailable',
    };
  }
  if (winners.length === 0 || available.length < 2) {
    return {
      validSample: false,
      winnerIds: winners,
      points: Object.fromEntries(MODEL_IDS.map((modelId) => [modelId, 0])),
      denominator: Object.fromEntries(MODEL_IDS.map((modelId) => [modelId, 0])),
      reason: winners.length === 0 ? 'none' : 'fewer_than_two_available_models',
    };
  }
  const points = Object.fromEntries(MODEL_IDS.map((modelId) => [modelId, 0]));
  const denominator = Object.fromEntries(MODEL_IDS.map((modelId) => [modelId, 0]));
  for (const modelId of available) denominator[modelId] = 1;
  const share = 1 / winners.length;
  for (const modelId of winners) points[modelId] = share;
  return { validSample: true, winnerIds: winners, points, denominator, reason: null };
}

export function createEvaluationSession(manifest) {
  // Rust has already applied the saved seed when it built the manifest. Keep
  // that order exactly so a resumed session cannot silently reshuffle tracks.
  const trackOrder = manifest.tracks.map((track) => track.trackId);
  const cards = Object.fromEntries(
    manifest.tracks.map((track) => [track.trackId, cardOrderForTrack(manifest.seed, track.trackId)]),
  );
  return {
    schemaVersion: 1,
    sessionId: manifest.sessionId,
    manifest,
    trackOrder,
    cards,
    cursor: 0,
    phase: 'human',
    answers: {},
    paused: false,
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
  };
}

export function summarizeSession(session) {
  const summary = {
    totalTracks: session.trackOrder.length,
    answeredTracks: 0,
    validSamples: 0,
    noneSelections: 0,
    points: Object.fromEntries(MODEL_IDS.map((modelId) => [modelId, 0])),
    denominators: Object.fromEntries(MODEL_IDS.map((modelId) => [modelId, 0])),
    byHumanLabel: {},
  };
  for (const answer of Object.values(session.answers ?? {})) {
    summary.answeredTracks += 1;
    const scored = scoreSelection(answer);
    if (answer.winnerIds === 'none') summary.noneSelections += 1;
    if (scored.validSample) summary.validSamples += 1;
    for (const modelId of MODEL_IDS) {
      summary.points[modelId] += scored.points[modelId];
      summary.denominators[modelId] += scored.denominator[modelId];
    }
    const label = answer.humanLabel ?? 'unknown';
    summary.byHumanLabel[label] ??= {
      count: 0,
      points: Object.fromEntries(MODEL_IDS.map((id) => [id, 0])),
    };
    summary.byHumanLabel[label].count += 1;
    for (const modelId of MODEL_IDS) {
      summary.byHumanLabel[label].points[modelId] += scored.points[modelId];
    }
  }
  summary.winRates = Object.fromEntries(MODEL_IDS.map((modelId) => [
    modelId,
    summary.denominators[modelId] > 0
      ? summary.points[modelId] / summary.denominators[modelId]
      : null,
  ]));
  return summary;
}

function csvEscape(value) {
  const text = value == null ? '' : String(value);
  return /[",\n]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text;
}

export function exportEvaluationJson(session) {
  return JSON.stringify({ ...session, summary: summarizeSession(session) }, null, 2);
}

export function exportEvaluationCsv(session) {
  const header = [
    'trackId', 'title', 'artist', 'clipStartSeconds', 'clipDurationSeconds',
    'clipSelection',
    'humanLabel', 'winnerIds', 'validSample', 'legacyMoodStatus',
    'emomusicStatus', 'museStatus', 'mirexStatus',
    'legacyMoodPoints', 'emomusicPoints', 'musePoints', 'mirexPoints',
    'legacyMoodDenominator', 'emomusicDenominator', 'museDenominator', 'mirexDenominator',
  ];
  const rows = [header.join(',')];
  for (const trackId of session.trackOrder) {
    const track = session.manifest.tracks.find((item) => item.trackId === trackId);
    const answer = session.answers?.[trackId];
    if (!track || !answer) continue;
    const scored = scoreSelection(answer);
    rows.push([
      track.trackId,
      track.title,
      track.artist,
      track.clipStartSeconds,
      track.clipDurationSeconds,
      track.clipSelection,
      answer.humanLabel,
      Array.isArray(answer.winnerIds) ? answer.winnerIds.join('|') : answer.winnerIds,
      scored.validSample,
      track.legacyMood?.status,
      track.emomusic?.status,
      track.muse?.status,
      track.mirex?.status,
      scored.points.legacyMood,
      scored.points.emomusic,
      scored.points.muse,
      scored.points.mirex,
      scored.denominator.legacyMood,
      scored.denominator.emomusic,
      scored.denominator.muse,
      scored.denominator.mirex,
    ].map(csvEscape).join(','));
  }
  return rows.join('\n');
}
