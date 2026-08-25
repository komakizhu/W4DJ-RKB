import { invoke } from '@tauri-apps/api/core';
import {
  analyzeAudioFile,
  ESSENTIA_MODEL_IDS,
  isCompleteTrackAnalysis,
  normalizeEssentiaModel,
  type EssentiaModelFile,
  type TrackAnalysis,
  type TrackMetadata,
} from './analysis';
import {
  AnalysisWorkerCancelledError,
  AnalysisWorkerClient,
  AnalysisWorkerTimeoutError,
} from './analysis-worker-client';
import type { HeadlessAcceptanceEvent } from './headless-acceptance';

export type LibraryAnalysisCandidate = {
  path: string;
  name: string;
  sizeBytes: number;
  slotIndex?: number;
};

export type LibraryAnalysisRunOptions = {
  runId: string;
  candidates: LibraryAnalysisCandidate[];
  resumeIncomplete: boolean;
  cancelAfterNewCompleted?: number;
  onEvent: (event: HeadlessAcceptanceEvent) => Promise<void> | void;
};

export type LibraryAnalysisRunResult = {
  total: number;
  completed: number;
  failed: number;
  timedOut: number;
  cancelled: number;
  pending: number;
};

type DesktopFingerprint = {
  sizeBytes: number;
  modifiedAt: number | null;
};

type PreviewCandidate = {
  name: string;
  source_path: string;
  destination_path: string;
  source_size_bytes: number;
  estimated_output_bytes: number | null;
  operation: 'update_metadata';
};

type SlotPreview = {
  slot_index: number;
  mode: 'compat' | 'lossless';
  lossless_format: 'wav' | 'aiff' | null;
  conflict_strategy: 'update_metadata';
  filename_rule: 'title_artist';
  retry_of: string | null;
  preview: {
    source_directory: string;
    destination_directory: string;
    new_count: number;
    existing_count: number;
    skipped_count: number;
    error_count: number;
    estimated_output_bytes: number | null;
    candidates: PreviewCandidate[];
    skipped: never[];
    errors: never[];
    warnings: never[];
    available_space_bytes: number | null;
    disk_space_sufficient: boolean | null;
  };
};

type HeadlessModelStatus = {
  embedding?: boolean;
};

function candidatePreview(candidate: LibraryAnalysisCandidate): SlotPreview {
  return {
    slot_index: candidate.slotIndex ?? 0,
    mode: 'compat',
    lossless_format: null,
    conflict_strategy: 'update_metadata',
    filename_rule: 'title_artist',
    retry_of: null,
    preview: {
      source_directory: '',
      destination_directory: '',
      new_count: 0,
      existing_count: 1,
      skipped_count: 0,
      error_count: 0,
      estimated_output_bytes: null,
      candidates: [{
        name: candidate.name,
        source_path: candidate.path,
        destination_path: candidate.path,
        source_size_bytes: candidate.sizeBytes,
        estimated_output_bytes: null,
        operation: 'update_metadata',
      }],
      skipped: [],
      errors: [],
      warnings: [],
      available_space_bytes: null,
      disk_space_sufficient: null,
    },
  };
}

async function loadModels(onEvent: LibraryAnalysisRunOptions['onEvent']): Promise<EssentiaModelFile[]> {
  let status: HeadlessModelStatus = {};
  try {
    // Headless analysis is itself an explicit enhanced-analysis request. Keep
    // startup lazy by validating/installing the bundled files only here.
    status = await invoke<HeadlessModelStatus>('ensure_essentia_models');
  } catch {
    // The worker can still provide a basic analysis when model status is not
    // available. The terminal report will expose the missing high-level data.
  }
  const models: EssentiaModelFile[] = [];
  for (const id of ESSENTIA_MODEL_IDS) {
    try {
      const model = await invoke<Parameters<typeof normalizeEssentiaModel>[0]>(
        'load_essentia_model',
        { id },
      );
      models.push(normalizeEssentiaModel(model));
      await onEvent({
        runId: '',
        scenario: 'libraryAnalysis',
        status: 'running',
        stage: 'loadingModels',
        message: `loaded ${id}`,
        timestampMs: Date.now(),
      });
    } catch (error) {
      await onEvent({
        runId: '',
        scenario: 'libraryAnalysis',
        status: status.embedding ? 'partial' : 'running',
        stage: 'loadingModels',
        message: `${id}: ${error instanceof Error ? error.message : String(error)}`,
        timestampMs: Date.now(),
      });
    }
  }
  return models;
}

function failureDetails(error: unknown): {
  message: string;
  status: 'failed' | 'timeout';
  stage?: string;
  elapsedMs?: number;
} {
  if (error instanceof AnalysisWorkerTimeoutError) {
    return {
      message: error.message,
      status: 'timeout',
      stage: error.stage,
      elapsedMs: error.elapsedMs,
    };
  }
  return {
    message: error instanceof Error ? error.message : String(error),
    status: 'failed',
  };
}

async function persistAnalysis(
  runId: string,
  preview: SlotPreview,
  analysis: TrackAnalysis | null,
  failure: ReturnType<typeof failureDetails> | null,
): Promise<void> {
  await invoke('apply_track_analysis_results', {
    batchId: runId,
    previews: [preview],
    analyses: analysis ? [analysis] : [],
    analysisFailures: failure
      ? [{
        path: preview.preview.candidates[0].source_path,
        message: failure.message,
        status: failure.status,
        stage: failure.stage,
        elapsedMs: failure.elapsedMs,
      }]
      : [],
  });
}

export async function runLibraryAnalysis(
  options: LibraryAnalysisRunOptions,
): Promise<LibraryAnalysisRunResult> {
  const uniqueCandidates = Array.from(
    new Map(options.candidates.map((candidate) => [candidate.path, candidate])).values(),
  );
  const total = uniqueCandidates.length;
  let completed = 0;
  let failed = 0;
  let timedOut = 0;
  let cancelled = 0;
  let newCompleted = 0;
  let processed = 0;
  let cancellationRequested = false;
  let lastProgressEventAt = 0;
  let lastProgressStage = '';
  let progressEventChain = Promise.resolve();
  const existing = new Map<string, TrackAnalysis>();

  try {
    const cached = await invoke<TrackAnalysis[]>('load_track_analyses');
    for (const entry of cached) existing.set(entry.path, entry);
  } catch {
    // The per-track writeback still remains authoritative when the legacy
    // compatibility JSON is unavailable.
  }

  await options.onEvent({
    runId: options.runId,
    scenario: 'libraryAnalysis',
    status: 'running',
    stage: 'preparing',
    processed: 0,
    total,
    message: `queued ${total} available tracks`,
    timestampMs: Date.now(),
  });

  const models = await loadModels(async (event) => options.onEvent({
    ...event,
    runId: options.runId,
  }));

  for (const candidate of uniqueCandidates) {
    if (options.resumeIncomplete && isCompleteTrackAnalysis(existing.get(candidate.path))) {
      completed += 1;
      processed += 1;
      await options.onEvent({
        runId: options.runId,
        scenario: 'libraryAnalysis',
        status: 'running',
        stage: 'skippingCompleted',
        processed,
        total,
        currentItem: candidate.name,
        message: 'existing completed result retained',
        timestampMs: Date.now(),
      });
      continue;
    }

    if (cancellationRequested) {
      cancelled += 1;
      continue;
    }

    const preview = candidatePreview(candidate);
    const worker = new AnalysisWorkerClient();
    const workerJobId = `${options.runId}-${processed + 1}`;
    const startedAt = Date.now();
    await options.onEvent({
      runId: options.runId,
      scenario: 'libraryAnalysis',
      status: 'running',
      stage: 'startingSong',
      processed,
      total,
      currentItem: candidate.name,
      message: 'starting per-song worker',
      timestampMs: startedAt,
    });

    try {
      lastProgressEventAt = 0;
      lastProgressStage = '';
      progressEventChain = Promise.resolve();
      await worker.start(workerJobId, models);
      await options.onEvent({
        runId: options.runId,
        scenario: 'libraryAnalysis',
        status: 'running',
        stage: 'readingAudio',
        processed,
        total,
        currentItem: candidate.name,
        timestampMs: Date.now(),
      });
      const bytes = Uint8Array.from(await invoke<number[]>('read_audio_file', { path: candidate.path }));
      let metadata: TrackMetadata | undefined;
      let fingerprint: DesktopFingerprint | undefined;
      try {
        metadata = await invoke<TrackMetadata>('read_audio_metadata', { path: candidate.path });
      } catch {
        metadata = undefined;
      }
      try {
        fingerprint = await invoke<DesktopFingerprint>('get_audio_file_fingerprint', { path: candidate.path });
      } catch {
        fingerprint = undefined;
      }
      const analysis = await analyzeAudioFile(candidate.path, bytes, metadata, {
        fingerprint,
        neteaseFilenameFormat: 'title_artist',
        highLevelModels: models,
        workerClient: worker,
        workerJobId,
        onProgress: (progress) => {
          const now = Date.now();
          const stageChanged = progress.stage !== lastProgressStage;
          if (!stageChanged && now - lastProgressEventAt < 1000) return;
          lastProgressEventAt = now;
          lastProgressStage = progress.stage;
          progressEventChain = progressEventChain
            .then(() => options.onEvent({
              runId: options.runId,
              scenario: 'libraryAnalysis',
              status: 'running',
              stage: progress.stage,
              processed,
              total,
              currentItem: candidate.name,
              message: progress.message,
              timestampMs: now,
            }))
            .catch(() => undefined);
        },
      });
      await progressEventChain;
      await options.onEvent({
        runId: options.runId,
        scenario: 'libraryAnalysis',
        status: 'running',
        stage: 'persisting',
        processed,
        total,
        currentItem: candidate.name,
        timestampMs: Date.now(),
      });
      await persistAnalysis(options.runId, preview, analysis, null);
      existing.set(candidate.path, analysis);
      completed += 1;
      newCompleted += 1;
      processed += 1;
      await options.onEvent({
        runId: options.runId,
        scenario: 'libraryAnalysis',
        status: 'running',
        stage: 'songCompleted',
        processed,
        total,
        currentItem: candidate.name,
        message: `completed in ${Date.now() - startedAt}ms`,
        timestampMs: Date.now(),
      });
      if (options.cancelAfterNewCompleted && newCompleted >= options.cancelAfterNewCompleted) {
        cancellationRequested = true;
        cancelled += 1;
        await options.onEvent({
          runId: options.runId,
          scenario: 'libraryAnalysis',
          status: 'cancelling',
          stage: 'cancelling',
          processed,
          total,
          currentItem: candidate.name,
          message: 'cooperative cancellation requested after persisted result',
          timestampMs: Date.now(),
        });
      }
    } catch (error) {
      if (error instanceof AnalysisWorkerCancelledError) {
        cancellationRequested = true;
        cancelled += 1;
      } else {
        const failure = failureDetails(error);
        try {
          await persistAnalysis(options.runId, preview, null, failure);
        } catch {
          // Preserve the original failure in the JSONL report even if the
          // SQLite writeback itself is unavailable.
        }
        failed += 1;
        if (failure.status === 'timeout') timedOut += 1;
        processed += 1;
        await options.onEvent({
          runId: options.runId,
          scenario: 'libraryAnalysis',
          status: 'partial',
          stage: failure.stage ?? 'failed',
          processed,
          total,
          currentItem: candidate.name,
          message: failure.message,
          timestampMs: Date.now(),
        });
      }
    } finally {
      worker.terminate();
    }
  }

  const pending = Math.max(0, total - processed);
  if (cancellationRequested) {
    await options.onEvent({
      runId: options.runId,
      scenario: 'libraryAnalysis',
      status: 'cancelled',
      stage: 'cancelled',
      processed,
      total,
      message: `${pending} tracks remain for resume`,
      timestampMs: Date.now(),
    });
  }
  return { total, completed, failed, timedOut, cancelled, pending };
}

export function summarizeLibraryAnalysisResult(result: LibraryAnalysisRunResult): 'completed' | 'partial' {
  return result.failed === 0 && result.timedOut === 0 && result.pending === 0 && result.cancelled === 0
    ? 'completed'
    : 'partial';
}
