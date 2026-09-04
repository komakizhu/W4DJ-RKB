/**
 * Runtime failures from Emscripten/WebAssembly are process-level failures in
 * the embedded WebKit content process. Keep this matcher deliberately narrow:
 * ordinary model/data errors must remain recoverable per song.
 */
export function isFatalAnalysisRuntimeMessage(message: string): boolean {
  return [
    /\babort\s*\(/i,
    /memory access out of bounds/i,
    /cannot enlarge memory/i,
    /out of memory|out-of-memory|oom/i,
    /wasm[^\n]*(?:trap|unreachable|abort|memory)/i,
    /runtimeerror[^\n]*(?:unreachable|memory|wasm|abort)/i,
    /webcontent[^\n]*(?:crash|terminat|reload|exit)/i,
  ].some((pattern) => pattern.test(message));
}

export function analysisErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    if (error.message && /runtimeerror|webassembly/i.test(error.name)) {
      return `${error.name}: ${error.message}`;
    }
    return error.message || error.name;
  }
  if (typeof error === 'string') return error;
  if (typeof error === 'number' || typeof error === 'bigint') return String(error);
  if (error && typeof error === 'object') {
    const record = error as Record<string, unknown>;
    const message = typeof record.message === 'string' ? record.message : '';
    const name = typeof record.name === 'string' ? record.name : '';
    if (message && /runtimeerror|webassembly/i.test(name)) return `${name}: ${message}`;
    if (message) return message;
    if (name) return name;
    try {
      return JSON.stringify(error);
    } catch {
      return Object.prototype.toString.call(error);
    }
  }
  return String(error);
}
