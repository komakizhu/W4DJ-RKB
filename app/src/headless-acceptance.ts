import { invoke } from '@tauri-apps/api/core';
import {
  runLibraryAnalysis,
  summarizeLibraryAnalysisResult,
  type LibraryAnalysisRunResult,
} from './library-analysis-runner';

export type HeadlessAcceptanceScenario =
  | 'libraryAnalysis'
  | 'neteaseMetadata'
  | 'flacCoverRecovery'
  | 'energyDashboard'
  | 'emotionManifest'
  | 'externalFormats'
  | 'bundleSmoke';

export type HeadlessAcceptanceRequest = {
  runId: string;
  scenario: HeadlessAcceptanceScenario;
  scope?: 'available';
  exerciseCancelResume?: boolean;
  inputPath?: string;
  outputPath?: string;
  databasePath?: string;
  reportPath: string;
};

export type HeadlessAcceptanceEvent = {
  runId: string;
  scenario: HeadlessAcceptanceScenario;
  status: 'starting' | 'running' | 'cancelling' | 'resuming' | 'completed' | 'partial' | 'blocked' | 'cancelled' | 'error';
  stage: string;
  processed?: number;
  total?: number;
  currentItem?: string;
  message?: string;
  timestampMs: number;
};

type HeadlessAcceptanceConfig = {
  scenario: HeadlessAcceptanceScenario;
  exerciseCancelResume: boolean;
  inputPath: string | null;
  outputPath: string | null;
  databasePath: string | null;
  reportPath: string;
};

type LibraryCandidate = {
  path: string;
  name: string;
  sizeBytes: number;
  slotIndex?: number;
};

function createRunId(): string {
  return `headless-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

async function writeEvent(
  config: HeadlessAcceptanceConfig,
  event: HeadlessAcceptanceEvent,
): Promise<void> {
  await invoke('write_headless_acceptance_event', {
    reportPath: config.reportPath,
    event,
  });
}

function combineResult(result: LibraryAnalysisRunResult, resumed: LibraryAnalysisRunResult): LibraryAnalysisRunResult {
  return {
    total: Math.max(result.total, resumed.total),
    completed: resumed.completed,
    failed: result.failed + resumed.failed,
    timedOut: result.timedOut + resumed.timedOut,
    cancelled: result.cancelled + resumed.cancelled,
    pending: resumed.pending,
  };
}

export function headlessExitCode(result: LibraryAnalysisRunResult): 0 | 2 {
  return result.failed === 0 && result.timedOut === 0 && result.pending === 0 && result.cancelled === 0
    ? 0
    : 2;
}

async function run(): Promise<void> {
  const config = await invoke<HeadlessAcceptanceConfig>('load_headless_acceptance_config');
  const runId = createRunId();
  const base = {
    runId,
    scenario: config.scenario,
  } as const;
  const emit = async (event: Omit<HeadlessAcceptanceEvent, 'runId' | 'scenario'>): Promise<void> => {
    await writeEvent(config, { ...base, ...event });
  };

  await emit({
    status: 'starting',
    stage: 'starting',
    message: 'headless acceptance runtime started',
    timestampMs: Date.now(),
  });

  if (config.scenario !== 'libraryAnalysis') {
    await emit({
      status: 'blocked',
      stage: 'unsupportedScenario',
      message: `${config.scenario} is not implemented in this runtime yet`,
      timestampMs: Date.now(),
    });
    await invoke('finish_headless_acceptance', { code: 3 });
    return;
  }

  const candidates = await invoke<LibraryCandidate[]>('list_library_analysis_candidates');
  if (candidates.length === 0) {
    await emit({
      status: 'blocked',
      stage: 'discoveringCandidates',
      processed: 0,
      total: 0,
      message: 'no readable available output files',
      timestampMs: Date.now(),
    });
    await invoke('finish_headless_acceptance', { code: 3 });
    return;
  }

  let firstResult: LibraryAnalysisRunResult;
  try {
    firstResult = await runLibraryAnalysis({
      runId,
      candidates,
      resumeIncomplete: false,
      cancelAfterNewCompleted: config.exerciseCancelResume ? 1 : undefined,
      onEvent: async (event) => {
        await writeEvent(config, {
          ...event,
          runId,
          scenario: config.scenario,
        });
      },
    });
  } catch (error) {
    await emit({
      status: 'error',
      stage: 'runnerError',
      message: error instanceof Error ? error.message : String(error),
      timestampMs: Date.now(),
    });
    await invoke('finish_headless_acceptance', { code: 4 });
    return;
  }

  let finalResult = firstResult;
  if (config.exerciseCancelResume && firstResult.cancelled > 0 && firstResult.pending > 0) {
    await emit({
      status: 'resuming',
      stage: 'resuming',
      processed: firstResult.completed,
      total: firstResult.total,
      message: 'resuming unfinished songs without rerunning completed results',
      timestampMs: Date.now(),
    });
    const resumed = await runLibraryAnalysis({
      runId,
      candidates,
      resumeIncomplete: true,
      onEvent: async (event) => {
        await writeEvent(config, {
          ...event,
          runId,
          scenario: config.scenario,
          status: event.status === 'running' ? 'resuming' : event.status,
        });
      },
    });
    finalResult = combineResult(firstResult, resumed);
  }

  const finalStatus = summarizeLibraryAnalysisResult(finalResult);
  await emit({
    status: finalStatus,
    stage: 'completed',
    processed: finalResult.total - finalResult.pending,
    total: finalResult.total,
    message: JSON.stringify(finalResult),
    timestampMs: Date.now(),
  });
  await invoke('finish_headless_acceptance', { code: headlessExitCode(finalResult) });
}

if (!(import.meta as ImportMeta & { vitest?: unknown }).vitest) {
  void run().catch(async (error) => {
  // If configuration cannot be read there is no safe report path to use.
  // The Rust command still supplies the non-zero process exit status.
  try {
    const config = await invoke<HeadlessAcceptanceConfig>('load_headless_acceptance_config');
    await writeEvent(config, {
      runId: createRunId(),
      scenario: config.scenario,
      status: 'error',
      stage: 'bootstrapError',
      message: error instanceof Error ? error.message : String(error),
      timestampMs: Date.now(),
    });
    await invoke('finish_headless_acceptance', { code: 4 });
  } catch {
    // The process will be terminated by the runtime when the hidden window
    // cannot be initialized; do not create a fallback report elsewhere.
  }
  });
}
