import type {
  AnalysisWorkerClientLike,
  DecodedAudioData,
  EssentiaModelFile,
  HighLevelAnalysis,
  NeteaseFilenameFormat,
  TrackAnalysis,
  TrackMetadata,
  AnalysisFingerprint,
} from './analysis';
import {
  type AnalysisWorkerProgress,
  type AnalysisWorkerRequest,
  type AnalysisWorkerResponse,
  serializeDecodedAudio,
  serializeEssentiaModels,
} from './analysis-worker-protocol';
import { analysisErrorMessage, isFatalAnalysisRuntimeMessage } from './analysis-runtime';
import { ANALYSIS_PROGRESS_STALL_TIMEOUT_MS } from './analysis-timeout';

export type AnalysisWorkerLike = {
  postMessage: (message: AnalysisWorkerRequest, transfer?: Transferable[]) => void;
  addEventListener: (type: 'message' | 'error', listener: EventListener) => void;
  removeEventListener: (type: 'message' | 'error', listener: EventListener) => void;
  terminate: () => void;
};

export type AnalysisWorkerFactory = () => AnalysisWorkerLike;

export class AnalysisWorkerCancelledError extends Error {
  constructor(message = '增强分析已取消') {
    super(message);
    this.name = 'AnalysisWorkerCancelledError';
  }
}

export class AnalysisWorkerClientError extends Error {
  readonly stage?: string;

  constructor(message: string, stage?: string) {
    super(message);
    this.name = 'AnalysisWorkerClientError';
    this.stage = stage;
  }
}

export class AnalysisWorkerFatalRuntimeError extends AnalysisWorkerClientError {
  readonly stage: string;
  readonly path?: string;
  readonly elapsedMs?: number;

  constructor(message: string, stage: string, path?: string, elapsedMs?: number) {
    super(message, stage);
    this.name = 'AnalysisWorkerFatalRuntimeError';
    this.stage = stage;
    this.path = path;
    this.elapsedMs = elapsedMs;
  }
}

export function isFatalAnalysisRuntimeError(error: unknown): boolean {
  return error instanceof AnalysisWorkerFatalRuntimeError
    || isFatalAnalysisRuntimeMessage(analysisErrorMessage(error));
}

export class AnalysisWorkerTimeoutError extends AnalysisWorkerClientError {
  readonly elapsedMs: number;
  readonly stage: string;
  readonly path: string;
  readonly modelId?: string;
  readonly modelFamily?: string;
  readonly lastProgressAt?: string;

  constructor(
    path: string,
    elapsedMs: number,
    stage: string,
    context: Pick<AnalysisWorkerProgress, 'modelId' | 'modelFamily' | 'stageStartedAt'> = {},
    lastProgressAt?: string,
  ) {
    const model = context.modelFamily || context.modelId
      ? `，模型：${context.modelFamily ?? ''}${context.modelId ? `/${context.modelId}` : ''}`
      : '';
    super(`增强分析超时：${path}（阶段：${stage}${model}，已等待 ${Math.round(elapsedMs / 1000)} 秒）`);
    this.name = 'AnalysisWorkerTimeoutError';
    this.path = path;
    this.elapsedMs = elapsedMs;
    this.stage = stage;
    this.modelId = context.modelId;
    this.modelFamily = context.modelFamily;
    this.lastProgressAt = lastProgressAt ?? context.stageStartedAt;
  }
}

type AnalysisRequest = {
  jobId: string;
  path: string;
  metadata?: TrackMetadata;
  fingerprint?: AnalysisFingerprint;
  neteaseFilenameFormat: NeteaseFilenameFormat;
  highLevel?: HighLevelAnalysis;
  audio: DecodedAudioData;
  onProgress?: (progress: AnalysisWorkerProgress) => void;
  timeoutMs?: number;
};

export type AnalysisWorkerSession = AnalysisWorkerClientLike & {
  start: (
    jobId: string,
    models: EssentiaModelFile[],
    tensorflowBackend?: 'cpu' | 'webgl' | 'wasm',
  ) => Promise<void>;
  terminate: (reason?: unknown) => void;
};

type PendingAnalysis = {
  resolve: (analysis: TrackAnalysis) => void;
  reject: (error: unknown) => void;
  onProgress?: (progress: AnalysisWorkerProgress) => void;
  totalTimer: ReturnType<typeof setTimeout>;
  stallTimer: ReturnType<typeof setTimeout>;
  startedAt: number;
  stage: string;
  path: string;
  lastProgress?: AnalysisWorkerProgress;
  lastProgressAt?: string;
};

const WORKER_START_TIMEOUT_MS = 120_000;

function defaultWorkerFactory(): AnalysisWorkerLike {
  const worker = new Worker(new URL('./analysis.worker.ts', import.meta.url), { type: 'module' });
  return {
    postMessage: (message, transfer = []) => worker.postMessage(message, transfer),
    addEventListener: (type, listener) => worker.addEventListener(type, listener),
    removeEventListener: (type, listener) => worker.removeEventListener(type, listener),
    terminate: () => worker.terminate(),
  };
}

function errorMessage(error: unknown): string {
  return analysisErrorMessage(error);
}

export class AnalysisWorkerClient implements AnalysisWorkerSession {
  private worker: AnalysisWorkerLike | null = null;
  private activeJobId: string | null = null;
  private readyPromise: Promise<void> | null = null;
  private readyResolve: (() => void) | null = null;
  private readyReject: ((error: unknown) => void) | null = null;
  private requestSequence = 0;
  private pending = new Map<string, PendingAnalysis>();
  private readonly factory: AnalysisWorkerFactory;
  private readonly handleMessage = (event: Event) => {
    const response = (event as MessageEvent<AnalysisWorkerResponse>).data;
    if (!response || response.jobId !== this.activeJobId) {
      return;
    }
    if (response.type === 'ready') {
      this.readyResolve?.();
      this.clearReadyPromise();
      return;
    }
    if (response.type === 'progress') {
      if (response.requestId) {
        const pending = this.pending.get(response.requestId);
        if (pending) {
          pending.stage = response.progress.stage;
          pending.lastProgress = response.progress;
          pending.lastProgressAt = new Date().toISOString();
          this.resetProgressWatchdog(response.requestId, pending);
          pending.onProgress?.(response.progress);
        }
      }
      return;
    }
    if (response.type === 'result') {
      const pending = this.pending.get(response.requestId);
      if (!pending) {
        return;
      }
      this.pending.delete(response.requestId);
      this.clearPendingTimers(pending);
      pending.resolve(response.analysis);
      return;
    }
    if (response.type === 'error') {
      if (response.requestId) {
        const pending = this.pending.get(response.requestId);
        if (!pending) {
          return;
        }
        this.pending.delete(response.requestId);
        this.clearPendingTimers(pending);
        const error = response.fatal || isFatalAnalysisRuntimeMessage(response.message)
          ? new AnalysisWorkerFatalRuntimeError(
            response.message,
            response.stage ?? pending.stage,
            pending.path,
            Date.now() - pending.startedAt,
          )
          : new AnalysisWorkerClientError(response.message, response.stage ?? pending.stage);
        pending.reject(error);
        if (error instanceof AnalysisWorkerFatalRuntimeError) {
          this.rejectPending(error);
          this.detachAndTerminate();
        }
      } else {
        const error = response.fatal || isFatalAnalysisRuntimeMessage(response.message)
          ? new AnalysisWorkerFatalRuntimeError(response.message, response.stage ?? 'loadingModels')
          : new AnalysisWorkerClientError(response.message, response.stage ?? 'loadingModels');
        this.readyReject?.(error);
        this.clearReadyPromise();
        this.rejectPending(error);
        if (error instanceof AnalysisWorkerFatalRuntimeError) {
          this.detachAndTerminate();
        }
      }
    }
  };
  private readonly handleError = (event: Event) => {
    const message = errorMessage(
      (event as ErrorEvent).error || (event as ErrorEvent).message || '分析 Worker 发生错误',
    );
    const pending = this.pending.values().next().value as PendingAnalysis | undefined;
    const error = isFatalAnalysisRuntimeMessage(message)
      ? new AnalysisWorkerFatalRuntimeError(
        message,
        pending?.stage ?? 'loadingModels',
        pending?.path,
        pending ? Date.now() - pending.startedAt : undefined,
      )
      : new AnalysisWorkerClientError(message, pending?.stage ?? 'loadingModels');
    this.readyReject?.(error);
    this.clearReadyPromise();
    this.rejectPending(error);
    this.detachAndTerminate();
  };

  constructor(factory: AnalysisWorkerFactory = defaultWorkerFactory) {
    this.factory = factory;
  }

  async start(
    jobId: string,
    models: EssentiaModelFile[],
    tensorflowBackend?: 'cpu' | 'webgl' | 'wasm',
  ): Promise<void> {
    if (this.worker && this.activeJobId === jobId && !this.readyPromise) {
      return;
    }
    this.terminate(new AnalysisWorkerCancelledError('分析任务已重新开始'));
    this.worker = this.factory();
    this.activeJobId = jobId;
    this.worker.addEventListener('message', this.handleMessage);
    this.worker.addEventListener('error', this.handleError);
    this.readyPromise = new Promise<void>((resolve, reject) => {
      this.readyResolve = resolve;
      this.readyReject = reject;
    });
    const serialized = serializeEssentiaModels(models);
    try {
      this.worker.postMessage({
        type: 'start',
        jobId,
        models: serialized.payload,
        tensorflowBackend,
      }, serialized.transfer);
    } catch (error) {
      const message = errorMessage(error);
      const wrapped = isFatalAnalysisRuntimeMessage(message)
        ? new AnalysisWorkerFatalRuntimeError(message, 'loadingModels')
        : new AnalysisWorkerClientError(message, 'loadingModels');
      this.readyReject?.(wrapped);
      this.clearReadyPromise();
      this.detachAndTerminate();
      throw wrapped;
    }
    const readyPromise = this.readyPromise;
    let timer: ReturnType<typeof setTimeout> | null = null;
    try {
      await Promise.race([
        readyPromise,
        new Promise<void>((_, reject) => {
          timer = setTimeout(() => {
            const error = new AnalysisWorkerTimeoutError(
              '<模型加载>',
              WORKER_START_TIMEOUT_MS,
              'loadingModels',
            );
            this.terminate(error);
            reject(error);
          }, WORKER_START_TIMEOUT_MS);
        }),
      ]);
    } finally {
      if (timer) {
        clearTimeout(timer);
      }
    }
  }

  analyze(request: AnalysisRequest): Promise<TrackAnalysis> {
    if (!this.worker || this.activeJobId !== request.jobId || this.readyPromise) {
      return Promise.reject(new AnalysisWorkerClientError('分析 Worker 尚未就绪'));
    }
    const requestId = `${request.jobId}-${this.requestSequence += 1}`;
    const serialized = serializeDecodedAudio(request.audio);
    return new Promise<TrackAnalysis>((resolve, reject) => {
      const startedAt = Date.now();
      const timeoutMs = request.timeoutMs ?? WORKER_START_TIMEOUT_MS;
      const totalTimer = setTimeout(() => {
        this.timeoutPending(requestId);
      }, Math.max(1, timeoutMs));
      const stallTimer = setTimeout(() => {
        this.timeoutPending(requestId);
      }, ANALYSIS_PROGRESS_STALL_TIMEOUT_MS);
      this.pending.set(requestId, {
        resolve,
        reject,
        onProgress: request.onProgress,
        totalTimer,
        stallTimer,
        startedAt,
        stage: 'preparing',
        path: request.path,
        lastProgressAt: new Date(startedAt).toISOString(),
      });
      try {
        this.worker?.postMessage({
          type: 'analyze',
          jobId: request.jobId,
          requestId,
          path: request.path,
          metadata: request.metadata,
          fingerprint: request.fingerprint,
          neteaseFilenameFormat: request.neteaseFilenameFormat,
          highLevel: request.highLevel,
          audio: serialized.payload,
        }, serialized.transfer);
      } catch (error) {
        const pending = this.pending.get(requestId);
        this.pending.delete(requestId);
        if (pending) {
          this.clearPendingTimers(pending);
        }
        const message = errorMessage(error);
        reject(isFatalAnalysisRuntimeMessage(message)
          ? new AnalysisWorkerFatalRuntimeError(message, 'preparing', request.path)
          : new AnalysisWorkerClientError(message, 'preparing'));
        if (isFatalAnalysisRuntimeMessage(message)) {
          this.detachAndTerminate();
        }
      }
    });
  }

  terminate(reason: unknown = new AnalysisWorkerCancelledError()): void {
    this.readyReject?.(reason);
    this.clearReadyPromise();
    this.rejectPending(reason);
    this.detachAndTerminate();
  }

  private timeoutPending(requestId: string): void {
    const pending = this.pending.get(requestId);
    if (!pending) {
      return;
    }
    this.pending.delete(requestId);
    const error = new AnalysisWorkerTimeoutError(
      pending.path,
      Date.now() - pending.startedAt,
      pending.stage,
      pending.lastProgress ?? {},
      pending.lastProgressAt,
    );
    this.clearPendingTimers(pending);
    pending.reject(error);
    this.terminate(error);
  }

  private clearPendingTimers(pending: PendingAnalysis): void {
    clearTimeout(pending.totalTimer);
    clearTimeout(pending.stallTimer);
  }

  private resetProgressWatchdog(requestId: string, pending: PendingAnalysis): void {
    clearTimeout(pending.stallTimer);
    pending.stallTimer = setTimeout(() => {
      this.timeoutPending(requestId);
    }, ANALYSIS_PROGRESS_STALL_TIMEOUT_MS);
  }

  private rejectPending(reason: unknown): void {
    const pending = Array.from(this.pending.values());
    this.pending.clear();
    pending.forEach((entry) => {
      this.clearPendingTimers(entry);
      entry.reject(reason);
    });
  }

  private clearReadyPromise(): void {
    this.readyPromise = null;
    this.readyResolve = null;
    this.readyReject = null;
  }

  private detachAndTerminate(): void {
    if (this.worker) {
      this.worker.removeEventListener('message', this.handleMessage);
      this.worker.removeEventListener('error', this.handleError);
      this.worker.terminate();
    }
    this.worker = null;
    this.activeJobId = null;
  }
}
