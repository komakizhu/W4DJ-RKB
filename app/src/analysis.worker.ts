import { analyzeDecodedAudio } from './analysis';
import {
  deserializeDecodedAudio,
  deserializeEssentiaModels,
  type AnalysisWorkerProgress,
  type AnalysisWorkerRequest,
  type AnalysisWorkerResponse,
} from './analysis-worker-protocol';
import { analysisErrorMessage, isFatalAnalysisRuntimeMessage } from './analysis-runtime';

type WorkerScope = {
  postMessage: (message: AnalysisWorkerResponse) => void;
  onmessage: ((event: MessageEvent<AnalysisWorkerRequest>) => void) | null;
};

const workerScope = globalThis as unknown as WorkerScope;
let activeJobId: string | null = null;
let models = [] as ReturnType<typeof deserializeEssentiaModels>;
let tensorflowBackend: 'cpu' | 'webgl' | 'wasm' | undefined;
let processing = false;
let currentStage = '';
let stageStartedAtMs = 0;

function post(message: AnalysisWorkerResponse): void {
  workerScope.postMessage(message);
}

function postProgress(
  jobId: string,
  requestId: string | undefined,
  progress: AnalysisWorkerProgress,
): void {
  const now = Date.now();
  if (progress.stage !== currentStage) {
    currentStage = progress.stage;
    stageStartedAtMs = now;
  }
  post({
    type: 'progress',
    jobId,
    requestId,
    progress: {
      ...progress,
      stageStartedAt: new Date(stageStartedAtMs).toISOString(),
      elapsedMs: Math.max(0, now - stageStartedAtMs),
    },
  });
}

function yieldToWorker(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

function postAnalysisError(
  jobId: string,
  requestId: string | undefined,
  error: unknown,
  fallbackStage = 'analyzingBasic',
): void {
  const message = analysisErrorMessage(error);
  post({
    type: 'error',
    jobId,
    requestId,
    stage: currentStage || fallbackStage,
    fatal: isFatalAnalysisRuntimeMessage(message),
    message,
  });
}

workerScope.onmessage = async (event) => {
  const request = event.data;
  if (request.type === 'start') {
    activeJobId = request.jobId;
    tensorflowBackend = request.tensorflowBackend;
    postProgress(request.jobId, undefined, {
      stage: 'loadingModels',
      message: request.models.length > 0 ? '正在准备 Essentia 预训练模型' : '正在准备 Essentia 分析',
    });
    try {
      models = deserializeEssentiaModels(request.models);
    } catch (error) {
      postAnalysisError(request.jobId, undefined, error, 'loadingModels');
      return;
    }
    await yieldToWorker();
    post({ type: 'ready', jobId: request.jobId });
    return;
  }

  if (request.jobId !== activeJobId) {
    return;
  }
  if (processing) {
    postAnalysisError(request.jobId, request.requestId, '分析 Worker 正在处理上一首歌曲');
    return;
  }

  processing = true;
  postProgress(request.jobId, request.requestId, {
    stage: 'analyzingBasic',
    message: '正在计算 BPM、Key 和响度',
  });
  await yieldToWorker();
  try {
    const analysis = await analyzeDecodedAudio(
      request.path,
      deserializeDecodedAudio(request.audio),
      request.metadata,
      {
        fingerprint: request.fingerprint,
        neteaseFilenameFormat: request.neteaseFilenameFormat,
        highLevel: request.highLevel,
        highLevelModels: models,
        tensorflowBackend,
        onProgress: (progress) => {
          postProgress(request.jobId, request.requestId, progress);
        },
      },
    );
    postProgress(request.jobId, request.requestId, {
      stage: 'completed',
      message: '当前歌曲分析完成',
    });
    post({
      type: 'result',
      jobId: request.jobId,
      requestId: request.requestId,
      analysis,
    });
  } catch (error) {
    postAnalysisError(request.jobId, request.requestId, error);
  } finally {
    processing = false;
  }
};
